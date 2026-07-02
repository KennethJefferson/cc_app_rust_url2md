# url2md — Real Markdown Conversion

**Date:** 2026-07-02
**Status:** Approved (pending spec review)

## Problem

`url2md` ships with a full CLI, parallel worker pool, progress bars, and stats — all
working. But its core conversion does not convert. `process_url_file()` reads a `.url`
file, trims it, and writes the **raw INI text** to a `.md` file with the extension
changed:

```rust
// src/main.rs, current behavior
fs::write(&item.output_path, trimmed)   // raw INI copied verbatim
```

So `github.url`:

```ini
[InternetShortcut]
URL=https://github.com
```

produces `github.md`:

```
[InternetShortcut]
URL=https://github.com
```

That is not Markdown — it is the source file renamed. Every doc (README, Usage,
Changelog, CLAUDE.md) promises the tool "extracts content and converts to Markdown."
This spec closes that gap.

## Scope

**In scope:** replace the write logic in `process_url_file()` with real conversion —
parse the `.url` file, extract the URL, emit a Markdown link. Everything is local text
transformation.

**Explicitly out of scope (per user):**
- No network access. URLs are **not** resolved, fetched, validated, or title-scraped.
  The label comes from the filename, not the page's `<title>`.
- No changes to CLI flags, the worker pool, channels, progress display, or stats.
- No new dependencies.

## Behavior

### Output format

Markdown inline link, label from the filename stem:

```
[<stem>](<url>)
```

For `github.url` containing `URL=https://github.com`:

```markdown
[github](https://github.com)
```

- **Label** = input file stem (`github.url` → `github`). `.url` files carry no title
  field, so the filename is the only human-meaningful name available.
- **URL** = the value of the first `URL=` line found (see parsing).
- A single trailing newline is written after the link line.

### Parsing rules

A `.url` file is an INI file. We do a minimal, forgiving line scan — no INI-parser
dependency:

1. Read the file to a string (already done).
2. Trim it. **If the trimmed content is empty → `Skipped { reason: "File is empty" }`.**
   (Unchanged from today. Empty in, nothing out.)
3. Scan lines for the first line whose key (case-insensitive, trimmed) is `URL`, of the
   form `URL=<value>`. Split on the **first** `=` only, so query strings containing `=`
   survive (`URL=https://x.com/?a=1&b=2` → `https://x.com/?a=1&b=2`).
4. Trim the extracted value.
   - **URL found and non-empty** → write `[<stem>](<value>)\n` →
     `Success`.
   - **No `URL=` line, OR the value is empty (`URL=`)** → **A3 fallback**: write the
     original trimmed text verbatim (today's behavior) → `Success`. Content is never
     lost.

### Decision table

| Input file state            | Result  | `.md` contents                    |
|-----------------------------|---------|-----------------------------------|
| Empty / whitespace only     | Skipped | *(no file written)*               |
| Has `URL=https://x`         | Success | `[stem](https://x)\n`             |
| Has `URL=` (empty value)    | Success | *raw trimmed text* (A3 fallback)  |
| No `URL=` line, but has text| Success | *raw trimmed text* (A3 fallback)  |
| Unreadable / write error    | Failed  | *(no file written)*               |

The empty→Skipped and error→Failed paths are already implemented and unchanged. Only the
"has content" branch gains real conversion + fallback.

## Design

One new pure function, one changed call site. Everything else in `main.rs` is untouched.

```rust
/// Extract the first URL= value from .url (INI) content.
/// Returns the trimmed URL if a non-empty `URL=<value>` line exists, else None.
fn extract_url(content: &str) -> Option<String> {
    content
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            if key.trim().eq_ignore_ascii_case("URL") {
                let v = value.trim();
                if v.is_empty() { None } else { Some(v.to_string()) }
            } else {
                None
            }
        })
}

/// Render the Markdown for a converted .url file.
fn to_markdown(stem: &str, url: &str) -> String {
    format!("[{}]({})\n", stem, url)
}
```

In `process_url_file()`, after the empty check, replace the single `fs::write(..., trimmed)`
with:

```rust
let body = match extract_url(&content) {
    Some(url) => {
        let stem = item.input_path.file_stem().unwrap_or_default().to_string_lossy();
        to_markdown(&stem, &url)
    }
    None => format!("{}\n", trimmed),   // A3 fallback: raw text, normalized trailing newline
};
// ... fs::write(&item.output_path, body) ...
```

`extract_url` and `to_markdown` are pure `&str → …` functions — trivially unit-testable
with no filesystem, no async.

### Why this shape

- **Pure core, thin shell.** All the logic that can be wrong (parsing, formatting) is in
  two pure functions. The async/IO wrapper stays dumb. This is the smallest change that
  makes the behavior correct and testable.
- **No INI crate.** `.url` files are trivially simple; a `split_once('=')` line scan is
  enough and adds zero dependencies. `split_once` (not `split`) preserves `=` in query
  strings.
- **Case-insensitive key** (`eq_ignore_ascii_case`) matches how the code already compares
  the `.url` extension, and real `.url` files are reliably `URL=` but we don't depend on
  casing.

## Testing

Add a `#[cfg(test)] mod tests` to `src/main.rs`. The pure functions make this clean —
no temp dirs, no tokio.

`extract_url`:
- standard `[InternetShortcut]\nURL=https://github.com` → `Some("https://github.com")`
- URL with query string containing `=` → full value preserved
- lowercase `url=` → matched (case-insensitive)
- `URL=` empty value → `None` (drives A3 fallback)
- no URL line (`[InternetShortcut]` only) → `None`
- URL not on the first line (icon/other keys before it) → found

`to_markdown`:
- `to_markdown("github", "https://github.com")` == `"[github](https://github.com)\n"`

Existing `test_data/*.md` fixtures (`github.md`, `google.md`) currently hold the *old*
raw-copy output. They will be **updated** to the new expected link output as part of
implementation, so they document real behavior:

```
github.md:  [github](https://github.com)
google.md:  [google](https://www.google.com)
```

## Verification

After implementing, the release binary is rebuilt and run against `test_data/` to confirm
real conversion end-to-end (not just unit tests):

```
url2md -i test_data -o <scratch>        # expect github.md/google.md as links, empty.url skipped
```

Exit code 0, summary shows 2 succeeded / 1 skipped / 0 failed.
