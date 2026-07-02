use anyhow::{Context, Result};
use async_channel::{bounded, Receiver};
use clap::Parser;
use console::Style;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::fs;
use tokio::sync::mpsc;
use walkdir::WalkDir;

// ============================================================================
// CLI Arguments
// ============================================================================

#[derive(Parser, Debug)]
#[command(name = "url2md")]
#[command(version = "0.1.0")]
#[command(about = "Extract text from Windows .url shortcut files and save as Markdown")]
#[command(after_help = r#"EXAMPLES:
  url2md -i file.url
  url2md -i "C:\urls\bookmark.url"
  url2md -i "C:\urls" -r
  url2md -i "[file1.url file2.url dir1]" -r -w 4
  url2md -i "[C:\urls D:\bookmarks]" -r -o "C:\output" -w 8
"#)]
struct Args {
    /// Input: file, directory, or bracketed list "[item1 item2 ...]"
    #[arg(short, long, required = true)]
    input: String,

    /// Recursive search (for directories only)
    #[arg(short, long, default_value = "false")]
    recursive: bool,

    /// Output directory (default: same location as input file)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Number of parallel workers (default: 1)
    #[arg(short, long, default_value = "1")]
    workers: usize,

    /// Verbose output
    #[arg(short, long, default_value = "false")]
    verbose: bool,
}

/// Parse bracket-enclosed space-separated paths or single path
fn parse_input(s: &str) -> Vec<PathBuf> {
    let trimmed = s.trim();

    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        // Bracketed list: "[item1 item2 item3]"
        let inner = &trimmed[1..trimmed.len() - 1];
        inner
            .split_whitespace()
            .map(PathBuf::from)
            .collect()
    } else {
        // Single item
        vec![PathBuf::from(trimmed)]
    }
}

// ============================================================================
// Styles
// ============================================================================

struct Styles {
    success: Style,
    warning: Style,
    error: Style,
    info: Style,
    header: Style,
    worker_styles: Vec<Style>,
}

impl Styles {
    fn new() -> Self {
        Self {
            success: Style::new().green().bold(),
            warning: Style::new().yellow(),
            error: Style::new().red().bold(),
            info: Style::new().cyan(),
            header: Style::new().white().bold(),
            worker_styles: vec![
                Style::new().magenta(),
                Style::new().cyan(),
                Style::new().blue(),
                Style::new().green(),
                Style::new().yellow(),
                Style::new().red(),
                Style::new().white(),
                Style::new().magenta().bold(),
            ],
        }
    }

    fn worker_style(&self, id: usize) -> &Style {
        &self.worker_styles[id % self.worker_styles.len()]
    }
}

// ============================================================================
// Statistics
// ============================================================================

struct Stats {
    files_processed: AtomicUsize,
    files_succeeded: AtomicUsize,
    files_skipped: AtomicUsize,
    files_failed: AtomicUsize,
}

impl Stats {
    fn new() -> Self {
        Self {
            files_processed: AtomicUsize::new(0),
            files_succeeded: AtomicUsize::new(0),
            files_skipped: AtomicUsize::new(0),
            files_failed: AtomicUsize::new(0),
        }
    }
}

// ============================================================================
// Work Items
// ============================================================================

struct WorkItem {
    input_path: PathBuf,
    output_path: PathBuf,
}

enum WorkResult {
    Success { input: PathBuf, output: PathBuf },
    Skipped { input: PathBuf, reason: String },
    Failed { input: PathBuf, error: String },
}

// ============================================================================
// File Discovery
// ============================================================================

fn discover_url_files(inputs: &[PathBuf], recursive: bool) -> Vec<PathBuf> {
    let mut files = Vec::new();

    for input in inputs {
        if input.is_file() {
            if input.extension().map_or(false, |ext| ext.eq_ignore_ascii_case("url")) {
                files.push(input.clone());
            }
        } else if input.is_dir() {
            if recursive {
                for entry in WalkDir::new(input)
                    .follow_links(true)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    let path = entry.path();
                    if path.is_file()
                        && path.extension().map_or(false, |ext| ext.eq_ignore_ascii_case("url"))
                    {
                        files.push(path.to_path_buf());
                    }
                }
            } else {
                if let Ok(entries) = std::fs::read_dir(input) {
                    for entry in entries.filter_map(|e| e.ok()) {
                        let path = entry.path();
                        if path.is_file()
                            && path.extension().map_or(false, |ext| ext.eq_ignore_ascii_case("url"))
                        {
                            files.push(path);
                        }
                    }
                }
            }
        }
    }

    files
}

fn determine_output_path(input: &PathBuf, output_dir: &Option<PathBuf>) -> PathBuf {
    let stem = input.file_stem().unwrap_or_default();
    let mut output_name = stem.to_os_string();
    output_name.push(".md");

    match output_dir {
        Some(dir) => dir.join(output_name),
        None => {
            let parent = input.parent().unwrap_or(std::path::Path::new("."));
            parent.join(output_name)
        }
    }
}

/// Extract the URL from .url (INI) content.
///
/// Returns the trimmed value of the FIRST line whose key is `URL`
/// (case-insensitive), or None if that value is empty or no `URL` line exists.
/// "First key wins": the scan stops at the first `URL` line, so a leading empty
/// `URL=` yields None (→ fallback) rather than reaching a later `URL=` line.
/// Borrows from `content` — no allocation.
fn extract_url(content: &str) -> Option<&str> {
    content
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            // Match on the KEY, not on a non-empty value: this is what stops the
            // scan at the first `URL` line and routes an empty one to the fallback.
            key.trim().eq_ignore_ascii_case("URL").then(|| value.trim())
        })
        .filter(|url| !url.is_empty())
}

/// Render the Markdown for a converted .url file: `[stem](url)\n`.
fn to_markdown(stem: &str, url: &str) -> String {
    format!("[{}]({})\n", stem, url)
}

// ============================================================================
// Worker
// ============================================================================

async fn worker(
    id: usize,
    work_rx: Receiver<WorkItem>,
    result_tx: mpsc::Sender<WorkResult>,
    spinner: ProgressBar,
    stats: Arc<Stats>,
    style: Style,
) {
    while let Ok(item) = work_rx.recv().await {
        let filename = item
            .input_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();

        spinner.set_message(format!(
            "{}: {}",
            style.apply_to(format!("Worker {}", id)),
            filename
        ));

        let result = process_url_file(&item).await;
        stats.files_processed.fetch_add(1, Ordering::Relaxed);

        match &result {
            WorkResult::Success { .. } => {
                stats.files_succeeded.fetch_add(1, Ordering::Relaxed);
            }
            WorkResult::Skipped { .. } => {
                stats.files_skipped.fetch_add(1, Ordering::Relaxed);
            }
            WorkResult::Failed { .. } => {
                stats.files_failed.fetch_add(1, Ordering::Relaxed);
            }
        }

        let _ = result_tx.send(result).await;
    }

    spinner.finish_and_clear();
}

async fn process_url_file(item: &WorkItem) -> WorkResult {
    // Read the URL file content
    let content = match fs::read_to_string(&item.input_path).await {
        Ok(c) => c,
        Err(e) => {
            return WorkResult::Failed {
                input: item.input_path.clone(),
                error: format!("Failed to read file: {}", e),
            };
        }
    };

    // Check if content is empty
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return WorkResult::Skipped {
            input: item.input_path.clone(),
            reason: "File is empty".to_string(),
        };
    }

    // Create output directory if needed
    if let Some(parent) = item.output_path.parent() {
        if !parent.exists() {
            if let Err(e) = fs::create_dir_all(parent).await {
                return WorkResult::Failed {
                    input: item.input_path.clone(),
                    error: format!("Failed to create output directory: {}", e),
                };
            }
        }
    }

    // Convert: link if a URL is present and we have a usable filename stem,
    // otherwise fall back to the raw trimmed text (normalized to one newline).
    let stem = item.input_path.file_stem().unwrap_or_default().to_string_lossy();
    let body = match extract_url(&content) {
        Some(url) if !stem.is_empty() => to_markdown(&stem, url),
        _ => format!("{}\n", trimmed),
    };

    // Write the markdown file
    match fs::write(&item.output_path, &body).await {
        Ok(_) => WorkResult::Success {
            input: item.input_path.clone(),
            output: item.output_path.clone(),
        },
        Err(e) => WorkResult::Failed {
            input: item.input_path.clone(),
            error: format!("Failed to write output file: {}", e),
        },
    }
}

// ============================================================================
// Main
// ============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let styles = Styles::new();
    let stats = Arc::new(Stats::new());

    // Print header
    println!();
    println!(
        "{}",
        styles.header.apply_to("═══════════════════════════════════════════════════════════")
    );
    println!(
        "  {} - URL Shortcut to Markdown Converter",
        styles.info.apply_to("url2md")
    );
    println!(
        "{}",
        styles.header.apply_to("═══════════════════════════════════════════════════════════")
    );
    println!();

    // Print configuration
    println!("{}", styles.header.apply_to("[1/3] Configuration"));
    println!(
        "  {} {}",
        styles.info.apply_to("Workers:"),
        args.workers
    );
    println!(
        "  {} {}",
        styles.info.apply_to("Recursive:"),
        if args.recursive { "yes" } else { "no" }
    );
    println!(
        "  {} {}",
        styles.info.apply_to("Verbose:"),
        if args.verbose { "yes" } else { "no" }
    );
    if let Some(ref out) = args.output {
        println!(
            "  {} {}",
            styles.info.apply_to("Output dir:"),
            out.display()
        );
    } else {
        println!(
            "  {} same as input",
            styles.info.apply_to("Output dir:")
        );
    }
    println!();

    // Parse input paths
    let input_paths = parse_input(&args.input);
    if input_paths.is_empty() {
        println!(
            "  {} No input paths specified",
            styles.error.apply_to("✗")
        );
        std::process::exit(1);
    }

    // Discover files
    println!("{}", styles.header.apply_to("[2/3] Discovering files..."));
    let url_files = discover_url_files(&input_paths, args.recursive);
    let total_files = url_files.len();

    if total_files == 0 {
        println!(
            "  {} No .url files found",
            styles.warning.apply_to("⚠")
        );
        return Ok(());
    }

    println!(
        "  {} Found {} .url file(s)",
        styles.success.apply_to("✓"),
        total_files
    );
    println!();

    // Create work items
    let work_items: Vec<WorkItem> = url_files
        .into_iter()
        .map(|input| {
            let output = determine_output_path(&input, &args.output);
            WorkItem {
                input_path: input,
                output_path: output,
            }
        })
        .collect();

    // Set up progress bars
    println!("{}", styles.header.apply_to("[3/3] Processing files..."));
    let multi = MultiProgress::new();

    // Main progress bar
    let main_pb = multi.add(ProgressBar::new(total_files as u64));
    main_pb.set_style(
        ProgressStyle::with_template(
            "  {spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
        )
        .unwrap()
        .progress_chars("█▓▒░"),
    );
    main_pb.enable_steady_tick(std::time::Duration::from_millis(100));

    // Worker spinners
    let num_workers = args.workers.min(total_files);
    let mut worker_spinners = Vec::with_capacity(num_workers);
    for i in 0..num_workers {
        let spinner = multi.add(ProgressBar::new_spinner());
        let color = match i % 8 {
            0 => "magenta",
            1 => "cyan",
            2 => "blue",
            3 => "green",
            4 => "yellow",
            5 => "red",
            6 => "white",
            _ => "magenta",
        };
        spinner.set_style(
            ProgressStyle::with_template(&format!(
                "  {{spinner:.{}}} {{msg}}",
                color
            ))
            .unwrap(),
        );
        spinner.enable_steady_tick(std::time::Duration::from_millis(80));
        spinner.set_message(format!(
            "{}: idle",
            styles.worker_style(i).apply_to(format!("Worker {}", i))
        ));
        worker_spinners.push(spinner);
    }

    // Set up channels
    let (work_tx, work_rx) = bounded::<WorkItem>(total_files);
    let (result_tx, mut result_rx) = mpsc::channel::<WorkResult>(total_files);

    // Spawn workers
    let mut handles = Vec::with_capacity(num_workers);
    for i in 0..num_workers {
        let rx = work_rx.clone();
        let tx = result_tx.clone();
        let spinner = worker_spinners[i].clone();
        let stats_clone = Arc::clone(&stats);
        let style = styles.worker_style(i).clone();

        handles.push(tokio::spawn(async move {
            worker(i, rx, tx, spinner, stats_clone, style).await;
        }));
    }
    drop(result_tx); // Drop original sender so channel closes when workers finish

    // Send work items
    for item in work_items {
        work_tx.send(item).await.context("Failed to send work item")?;
    }
    drop(work_tx); // Close work channel

    // Collect results and update progress
    let mut results = Vec::new();
    while let Some(result) = result_rx.recv().await {
        main_pb.inc(1);
        results.push(result);
    }

    // Wait for all workers
    for handle in handles {
        let _ = handle.await;
    }

    main_pb.finish_and_clear();
    for spinner in worker_spinners {
        spinner.finish_and_clear();
    }

    // Print summary
    println!();
    println!(
        "{}",
        styles.header.apply_to("═══════════════════════════════════════════════════════════")
    );
    println!("  {}", styles.header.apply_to("Summary"));
    println!(
        "{}",
        styles.header.apply_to("═══════════════════════════════════════════════════════════")
    );

    let succeeded = stats.files_succeeded.load(Ordering::Relaxed);
    let skipped = stats.files_skipped.load(Ordering::Relaxed);
    let failed = stats.files_failed.load(Ordering::Relaxed);

    println!(
        "  {} Succeeded: {}",
        styles.success.apply_to("✓"),
        succeeded
    );
    if skipped > 0 {
        println!(
            "  {} Skipped:   {}",
            styles.warning.apply_to("⚠"),
            skipped
        );
    }
    if failed > 0 {
        println!(
            "  {} Failed:    {}",
            styles.error.apply_to("✗"),
            failed
        );
    }
    println!();

    // Print details for skipped/failed (or all if verbose)
    for result in &results {
        match result {
            WorkResult::Success { input, output } => {
                if args.verbose {
                    println!(
                        "  {} {} -> {}",
                        styles.success.apply_to("✓"),
                        input.display(),
                        output.display()
                    );
                }
            }
            WorkResult::Skipped { input, reason } => {
                println!(
                    "  {} {} - {}",
                    styles.warning.apply_to("⚠"),
                    input.display(),
                    reason
                );
            }
            WorkResult::Failed { input, error } => {
                println!(
                    "  {} {} - {}",
                    styles.error.apply_to("✗"),
                    input.display(),
                    error
                );
            }
        }
    }

    if failed > 0 {
        std::process::exit(1);
    }

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::{extract_url, to_markdown};

    #[test]
    fn extract_url_standard() {
        assert_eq!(
            extract_url("[InternetShortcut]\nURL=https://github.com"),
            Some("https://github.com")
        );
    }

    #[test]
    fn extract_url_preserves_query_string_with_equals() {
        assert_eq!(
            extract_url("URL=https://x.com/?a=1&b=2"),
            Some("https://x.com/?a=1&b=2")
        );
    }

    #[test]
    fn extract_url_is_case_insensitive() {
        assert_eq!(extract_url("url=https://x.com"), Some("https://x.com"));
    }

    #[test]
    fn extract_url_trims_whitespace_around_key_and_value() {
        assert_eq!(extract_url("  URL  =  https://x.com  "), Some("https://x.com"));
    }

    #[test]
    fn extract_url_empty_value_is_none() {
        assert_eq!(extract_url("URL="), None);
    }

    #[test]
    fn extract_url_no_url_line_is_none() {
        assert_eq!(extract_url("[InternetShortcut]\nIconIndex=0"), None);
    }

    #[test]
    fn extract_url_finds_url_not_on_first_line() {
        assert_eq!(
            extract_url("[InternetShortcut]\nIconIndex=0\nURL=https://x.com"),
            Some("https://x.com")
        );
    }

    // The regression the Codex review caught: a leading empty `URL=` must NOT
    // fall through to a later `URL=` line. First URL key wins.
    #[test]
    fn extract_url_first_key_wins_even_when_empty() {
        assert_eq!(extract_url("URL=\nURL=https://real.com"), None);
    }

    #[test]
    fn to_markdown_formats_link_with_trailing_newline() {
        assert_eq!(
            to_markdown("github", "https://github.com"),
            "[github](https://github.com)\n"
        );
    }
}
