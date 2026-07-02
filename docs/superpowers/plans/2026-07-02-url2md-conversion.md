# url2md Real Markdown Conversion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace url2md's raw-copy write logic with real conversion that turns a `.url` shortcut into a Markdown link `[stem](url)`, falling back to raw text when no URL is present.

**Architecture:** Two pure `&str` functions (`extract_url`, `to_markdown`) added to `src/main.rs`, plus a changed write branch in `process_url_file()`. All parsing/formatting logic is pure and unit-tested; the async IO wrapper stays dumb. No new dependencies, no CLI/worker/channel/progress/stats changes.

**Tech Stack:** Rust 2021, existing crates only (tokio, clap, indicatif, console, walkdir, anyhow). Tests are plain `#[cfg(test)]` unit tests — no tokio-test, no temp dirs.

## Global Constraints

- Rust 2021 edition; no new dependencies (verbatim from spec: "No new dependencies").
- `cargo` is NOT on PATH in this environment. Every cargo command MUST be prefixed to put it on PATH for that invocation. In PowerShell: `$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"; cargo ...`. In Bash: `PATH="$HOME/.cargo/bin:$PATH" cargo ...`.
- The repo lives on a UNC share (`\\LAPTOP-CHRISTINAMILLIAN\...`). A stale `.git/index.lock` (zero bytes, no live git process) can be left behind after a commit. If `git commit` fails with "Unable to create '.../index.lock': File exists", verify no git process is running, then `Remove-Item ".git\index.lock" -Force` and retry. This is expected, not corruption.
- Output format is exactly `[<stem>](<url>)` followed by a single `\n`. Label = input file stem. First `URL`-key line wins. Empty value or empty stem or no URL line → A3 fallback (raw trimmed text + one `\n`). Empty/whitespace-only input → Skipped (unchanged). These values are copied verbatim from the design spec at `docs/superpowers/specs/2026-07-02-url2md-conversion-design.md`.
- Commit messages end with the trailer:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

---

### Task 1: Add `extract_url` and `to_markdown` pure functions (with unit tests)

Add the two pure functions and their unit tests. This task produces the tested conversion core but does NOT yet wire it into `process_url_file()` — the functions will be `#[allow(dead_code)]`-clean because the tests reference them, so the build stays green. Task 2 wires them in and removes any unused-warning risk.

**Files:**
- Modify: `src/main.rs` — insert both functions immediately before `async fn process_url_file` (currently line 247), i.e. right after `determine_output_path` ends (line 199) is also acceptable; place them in the free-function region above `process_url_file`.
- Modify: `src/main.rs` — append a `#[cfg(test)] mod tests` at the end of the file (after the final line, currently 553).

**Interfaces:**
- Consumes: nothing (pure functions, no earlier-task dependencies).
- Produces:
  - `fn extract_url(content: &str) -> Option<&str>` — returns the trimmed value of the FIRST line whose key (case-insensitive, trimmed) is `URL`, or `None` if that value is empty or no such line exists. Borrows from `content`.
  - `fn to_markdown(stem: &str, url: &str) -> String` — returns `format!("[{}]({})\n", stem, url)`.

- [ ] **Step 1: Write the failing tests**

Append this module to the very end of `src/main.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail to compile**

Run: `$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"; cargo test 2>&1 | Select-Object -Last 20`

Expected: compile error — `cannot find function \`extract_url\`` and `cannot find function \`to_markdown\`` in module `super` (the functions don't exist yet).

- [ ] **Step 3: Write minimal implementation**

Insert these two functions into `src/main.rs` immediately before `async fn process_url_file(item: &WorkItem) -> WorkResult {` (currently line 247):

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"; cargo test 2>&1 | Select-Object -Last 20`

Expected: `test result: ok. 9 passed; 0 failed` (the 9 tests in `mod tests`). The binary itself is unchanged so far.

- [ ] **Step 5: Commit**

```bash
PATH="$HOME/.cargo/bin:$PATH" git add src/main.rs && git commit -m "$(cat <<'EOF'
Add extract_url and to_markdown pure functions with tests

First-URL-key-wins parsing: an empty leading URL= routes to fallback
rather than scanning to a later URL= line. Not yet wired into
process_url_file (Task 2).

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

If the commit fails on a stale `.git/index.lock`, see Global Constraints, remove the lock, and retry.

---

### Task 2: Wire conversion into `process_url_file` and update fixtures

Replace the raw-copy write with real conversion, and update the two `.md` fixtures to the new expected output. After this task the binary actually converts.

**Files:**
- Modify: `src/main.rs:280-281` — the `// Write the content to the markdown file` comment and the `match fs::write(&item.output_path, trimmed).await {` line. Insert the `body` computation before the match and change `trimmed` → `body` in the write call.
- Modify: `test_data/github.md` — replace old raw-copy contents with `[github](https://github.com)` + newline.
- Modify: `test_data/google.md` — replace with `[google](https://www.google.com)` + newline.

**Interfaces:**
- Consumes: `extract_url(&str) -> Option<&str>` and `to_markdown(&str, &str) -> String` from Task 1.
- Produces: no new symbols; changes the runtime behavior of `process_url_file` (the `Success` path now writes converted Markdown).

- [ ] **Step 1: Update the fixtures to the expected converted output**

These fixtures currently hold the OLD raw-copy output (`[InternetShortcut]\r\nURL=...`, CRLF, no trailing newline). Overwrite each with the new link output. Use Write (not Edit) to replace the whole file.

`test_data/github.md` — one line:
```
[github](https://github.com)
```

`test_data/google.md` — one line:
```
[google](https://www.google.com)
```

IMPORTANT — line-ending reality (verified): this repo has `core.autocrlf=true` and no `.gitattributes`. Git will rewrite these `.md` files to CRLF on checkout, and the running binary writes LF (`\n`). Therefore **the on-disk fixtures are human-readable references, NOT byte-exact oracles** — do not build the exact-match assertion in Step 4 against the fixture file's bytes, because autocrlf guarantees the trailing/line bytes will differ from the binary's LF output. Step 4 asserts the binary's output against a literal expected string instead. Just make each fixture contain the single visible line above; its exact trailing bytes don't matter for correctness.

- [ ] **Step 2: Change the write branch to compute `body` and write it**

In `src/main.rs`, replace this exact block (currently lines 280–281):

```rust
    // Write the content to the markdown file
    match fs::write(&item.output_path, trimmed).await {
```

with:

```rust
    // Convert: link if a URL is present and we have a usable filename stem,
    // otherwise fall back to the raw trimmed text (normalized to one newline).
    let stem = item.input_path.file_stem().unwrap_or_default().to_string_lossy();
    let body = match extract_url(&content) {
        Some(url) if !stem.is_empty() => to_markdown(&stem, url),
        _ => format!("{}\n", trimmed),
    };

    // Write the markdown file
    match fs::write(&item.output_path, &body).await {
```

Note: `fs::write` takes `impl AsRef<[u8]>`; `&String` satisfies it. `content` is still in scope (owned `String` from the read at the top of the function), so `extract_url(&content)` borrows validly, and `trimmed` (a `&str` into `content`) is still valid for the fallback.

- [ ] **Step 3: Verify it compiles and unit tests still pass**

Run: `$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"; cargo test 2>&1 | Select-Object -Last 20`

Expected: compiles with no warnings; `test result: ok. 9 passed; 0 failed`. (No `unused function` warning for `extract_url`/`to_markdown` now that `process_url_file` calls them.)

- [ ] **Step 4: End-to-end check against the real fixtures**

Build release and run the binary against `test_data`, sending output to a scratch dir so the repo fixtures aren't touched by the run. Then diff the produced files against the committed fixtures.

Run (PowerShell):
```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo build --release 2>&1 | Select-Object -Last 3
$scratch = Join-Path $env:TEMP ("url2md_e2e_" + [System.Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $scratch | Out-Null
& .\target\release\url2md.exe -i test_data -o $scratch
$exit = $LASTEXITCODE
# Read the binary's fresh output as raw bytes (LF, untouched by Git/autocrlf).
$prodG = Get-Content -Raw (Join-Path $scratch 'github.md')
$prodO = Get-Content -Raw (Join-Path $scratch 'google.md')
"exit code: $exit"
"github.md == expected: $($prodG -ceq \"[github](https://github.com)`n\")"
"google.md == expected: $($prodO -ceq \"[google](https://www.google.com)`n\")"
# empty.url must not have produced an output file at all
"empty.md absent (skipped): $(-not (Test-Path (Join-Path $scratch 'empty.md')))"
Remove-Item -Recurse -Force $scratch
```

Expected — every line True / as noted:
- Summary shows `Succeeded: 2` and `Skipped: 1` (the `empty.url` file), `Failed: 0`.
- `exit code: 0`
- `github.md == expected: True` — the binary's output is exactly `[github](https://github.com)` + one LF. This asserts against a literal string, NOT the on-disk fixture, because autocrlf would rewrite the fixture's newline (see Step 1).
- `google.md == expected: True`
- `empty.md absent (skipped): True` — confirms the empty-file skip still writes nothing.

If `github.md == expected` is `False`, print `$prodG | Format-Hex` to see the actual bytes: the likely culprit is a missing or doubled trailing newline in `to_markdown`, or a stray CR — fix the source, not the test.

- [ ] **Step 5: Commit**

```bash
PATH="$HOME/.cargo/bin:$PATH" git add src/main.rs test_data/github.md test_data/google.md && git commit -m "$(cat <<'EOF'
Convert .url files to Markdown links in process_url_file

Replace raw-copy write with [stem](url) conversion + raw-text fallback.
Update github.md/google.md fixtures from old raw-copy output to the new
link output. Verified end-to-end against test_data (2 succeeded, 1
skipped, exit 0).

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

If the commit fails on a stale `.git/index.lock`, see Global Constraints, remove the lock, and retry.

---

### Task 3: Update docs to describe real conversion

The README and Changelog describe the tool as "extracting content" but never show the actual output format. Now that conversion is real, document the output shape and the fallback so the docs match behavior. This is the last task; it has no code and no test cycle beyond a docs proofread, but it is a reviewer-gateable deliverable (a fresh reviewer could approve Task 2 yet reject wording here).

**Files:**
- Modify: `README.md` — add an "Output" example under Features/Quick Start.
- Modify: `Changelog.md` — add an entry under `[0.1.0]` (or a new unreleased section) noting real conversion.

**Interfaces:**
- Consumes: nothing.
- Produces: nothing (documentation only).

- [ ] **Step 1: Add an Output section to `README.md`**

Insert this block immediately after the closing ``` of the "Quick Start" fenced code block in `README.md` (after the multiple-inputs example, before the `## Requirements` heading):

```markdown
## Output

Each `.url` file becomes a Markdown link named after the file:

Input `github.url`:
```
[InternetShortcut]
URL=https://github.com
```

Output `github.md`:
```
[github](https://github.com)
```

If a `.url` file has no `URL=` line, its text is written through unchanged
(nothing is lost). Empty files are skipped.
```

- [ ] **Step 2: Add a Changelog entry**

In `Changelog.md`, under the `## [0.1.0] - 2026-01-20` `### Added` list, append this bullet:

```markdown
- Convert `.url` shortcuts to Markdown links (`[filename](url)`), with raw-text fallback when no URL is present
```

- [ ] **Step 3: Proofread**

Run: `$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"; cargo build 2>&1 | Select-Object -Last 3`

Expected: still builds (docs-only change, sanity check that nothing else was touched). Visually confirm the README fenced blocks are balanced (each ``` has a partner).

- [ ] **Step 4: Commit**

```bash
PATH="$HOME/.cargo/bin:$PATH" git add README.md Changelog.md && git commit -m "$(cat <<'EOF'
Document real Markdown conversion output

Show the [filename](url) output format and the raw-text fallback in the
README, and note conversion in the changelog.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

If the commit fails on a stale `.git/index.lock`, see Global Constraints, remove the lock, and retry.

---

## Notes for the implementer

- **Why `.find_map(...).filter(...)` and not the emptiness check inside `find_map`:** `find_map` returns at the first `Some`. If the closure returned `None` for an empty value, the iterator would keep scanning and pick up a *later* `URL=` line — the exact bug the review caught. So the `URL`-key match must be what produces `Some`; emptiness is filtered *after* the first match is chosen. Task 1's `extract_url_first_key_wins_even_when_empty` test locks this in — do not "simplify" it away.
- **Do not add** section-awareness (`[InternetShortcut]` scoping), Markdown-escaping, or BOM handling. The spec explicitly declines these. Adding them is out of scope.
- **`to_string_lossy()` returns a `Cow<str>`.** `stem.is_empty()` and `&stem` (deref to `&str`) both work directly on it; no extra `.to_string()` needed.
