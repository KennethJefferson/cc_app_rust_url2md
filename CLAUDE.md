# CLAUDE.md - Project Guidelines for AI Assistants

## Project Overview
url2md is a Rust-based CLI tool for converting Windows .url shortcut files to Markdown format.

## Build Commands
```bash
cargo build           # Debug build
cargo build --release # Release build
cargo run -- [args]   # Run with arguments
cargo test            # Run tests
```

## Architecture
- Single-file application (`src/main.rs`)
- Async runtime: Tokio
- CLI parsing: Clap with derive macros
- Progress display: Indicatif + Console
- Directory traversal: WalkDir
- Error handling: Anyhow

## Key Components
- `Args`: CLI argument structure
- `WorkItem`/`WorkResult`: Work queue items
- `worker()`: Async worker processing files
- `discover_url_files()`: File discovery with optional recursion
- `process_url_file()`: Core conversion logic

## Code Style
- Use Rust 2021 edition idioms
- Prefer `anyhow::Result` for error handling
- Keep atomic operations for stats counters
- Use async channels for work distribution

## Testing
Test files located in `test_data/` directory.
