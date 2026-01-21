# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-01-20

### Added
- Initial release
- Convert Windows .url shortcut files to Markdown
- Support for single file, directory, and bracketed list inputs
- Recursive directory scanning with `-r` flag
- Parallel processing with configurable worker count (`-w`)
- Custom output directory support (`-o`)
- Progress bars with per-worker status display
- Colored terminal output
- Summary statistics (succeeded/skipped/failed counts)
