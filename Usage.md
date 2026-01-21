# Usage Guide

## Command Syntax

```
url2md [OPTIONS] -i <INPUT>
```

## Options

| Option | Short | Description | Default |
|--------|-------|-------------|---------|
| `--input` | `-i` | Input file, directory, or bracketed list | Required |
| `--recursive` | `-r` | Enable recursive directory scanning | false |
| `--output` | `-o` | Output directory for converted files | Same as input |
| `--workers` | `-w` | Number of parallel workers | 1 |
| `--help` | `-h` | Show help message | - |
| `--version` | `-V` | Show version | - |

## Input Formats

### Single File
```bash
url2md -i "bookmark.url"
url2md -i "C:\Users\Tony\bookmarks\site.url"
```

### Single Directory
```bash
url2md -i "C:\bookmarks"
url2md -i "C:\bookmarks" -r  # recursive
```

### Multiple Inputs (Bracketed List)
```bash
url2md -i "[file1.url file2.url]"
url2md -i "[C:\urls D:\bookmarks E:\links]" -r
```

## Examples

### Basic Conversion
Convert a single URL file (output goes to same directory):
```bash
url2md -i "C:\bookmarks\favorite.url"
# Creates: C:\bookmarks\favorite.md
```

### Batch Processing
Convert all URL files in a directory:
```bash
url2md -i "C:\bookmarks" -r -w 4
```

### Custom Output Directory
Send all converted files to a specific location:
```bash
url2md -i "C:\bookmarks" -r -o "C:\markdown_exports" -w 8
```

### Multiple Sources
Process multiple directories at once:
```bash
url2md -i "[C:\work_bookmarks D:\personal_bookmarks]" -r -o "C:\all_bookmarks" -w 4
```

## Output

The tool displays:
1. Configuration summary
2. File discovery progress
3. Processing progress with per-worker status
4. Final summary with success/skip/fail counts

### Exit Codes
- `0`: All files processed successfully
- `1`: One or more files failed to process
