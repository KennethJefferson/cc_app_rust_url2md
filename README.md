# url2md

A fast, parallel CLI tool for extracting content from Windows .url shortcut files and saving them as Markdown files.

## Features

- Convert single .url files or entire directories
- Recursive directory scanning
- Parallel processing with configurable worker count
- Progress bars and colored output
- Preserves directory structure when using output directory

## Installation

```bash
cargo install --path .
```

Or build from source:

```bash
cargo build --release
```

The binary will be at `target/release/url2md.exe`

## Quick Start

```bash
# Convert a single file
url2md -i file.url

# Convert all .url files in a directory
url2md -i "C:\urls"

# Recursive scan with 4 workers
url2md -i "C:\urls" -r -w 4

# Multiple inputs with custom output directory
url2md -i "[C:\urls D:\bookmarks]" -r -o "C:\output" -w 8
```

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

## Requirements

- Windows (for .url shortcut files)
- Rust 1.70+ (for building)

## License

MIT
