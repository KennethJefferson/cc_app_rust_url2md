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
  field, so the filename is the only human-meaningful name available. If the stem is
  empty (e.g. a dotfile-style name), fall back to raw text (see parsing, step 4).
- **URL** = the value of the **first line whose key is `URL`** (see parsing). "First key
  wins" — a later `URL=` line is never consulted, even if the first one is empty.
- A single trailing newline is written after the link line.

### Parsing rules

A `.url` file is an INI file. We do a minimal, forgiving line scan — no INI-parser
dependency:

1. Read the file to a string (already done).
2. Trim it. **If the trimmed content is empty → `Skipped { reason: "File is empty" }`.**
   (Unchanged from today. Empty in, nothing out.)
3. Scan lines for the **first line whose key** (case-insensitive, trimmed) is `URL`,
   splitting on the **first** `=` only so query strings containing `=` survive
   (`URL=https://x.com/?a=1&b=2` → `https://x.com/?a=1&b=2`). Matching is keyed on the
   `URL` **key**, not on finding a non-empty value: once the first `URL` line is seen, the
   scan stops there and its value decides the outcome. This means a leading empty `URL=`
   line routes to the fallback rather than silently picking up a later `URL=` line — the
   code and this rule must agree (see `extract_url` below).
4. Decide from that first `URL` line's trimmed value (or its absence), plus the stem:
   - **`URL` line found, value non-empty, stem non-empty** → write `[<stem>](<value>)\n`
     → `Success`.
   - **No `URL` line at all, OR its value is empty (`URL=`), OR the stem is empty** →
     **A3 fallback**: write the original trimmed text plus a single trailing newline →
     `Success`. Content is never lost.

### Decision table

| Input file state                     | Result  | `.md` contents                         |
|--------------------------------------|---------|----------------------------------------|
| Empty / whitespace only              | Skipped | *(no file written)*                    |
| First `URL` line has non-empty value | Success | `[stem](value)\n`                      |
| First `URL` line is empty (`URL=`)   | Success | *raw trimmed text* + `\n` (A3 fallback)|
| No `URL` line, but has text          | Success | *raw trimmed text* + `\n` (A3 fallback)|
| `URL` found but stem is empty        | Success | *raw trimmed text* + `\n` (A3 fallback)|
| Unreadable / write error             | Failed  | *(no file written)*                    |

The empty→Skipped and error→Failed paths are already implemented and unchanged. Only the
"has content" branch gains real conversion + fallback. Note the fallback normalizes the
trailing newline to exactly one — it is "raw trimmed text", not a byte-for-byte copy of
the input.

## Design

One new pure function, one changed call site. Everything else in `main.rs` is untouched.

```rust
/// Extract the URL from .url (INI) content.
///
/// Returns the trimmed value of the FIRST line whose key is `URL`
/// (case-insensitive), or None if that value is empty or no `URL` line exists.
/// "First key wins": the scan stops at the first `URL` line, so a leading empty
/// `URL=` yields None (→ fallback) rather than reaching a later `URL=` line.
/// Borrows from `content` — no allocation.
fn extract_url(content: &str) -> Option<&str> {
    content.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        // Match on the KEY, not on a non-empty value: this is what stops the
        // scan at the first `URL` line and routes an empty one to the fallback.
        key.trim().eq_ignore_ascii_case("URL").then(|| value.trim())
    })
    .filter(|url| !url.is_empty())
}

/// Render the Markdown for a converted .url file.
fn to_markdown(stem: &str, url: &str) -> String {
    format!("[{}]({})\n", stem, url)
}
```

Why `.find_map(...).filter(...)` and not the value-check inside `find_map`: `find_map`
returns at the first `Some`, so the `URL`-key check must be the thing that produces
`Some` — otherwise an empty first `URL=` returns `None` from the closure and the iterator
keeps scanning to the next `URL=` line, which is the bug the review caught. Emptiness is
therefore filtered *after* the first match is chosen, not used to reject the match.

In `process_url_file()`, after the empty check, replace the single `fs::write(..., trimmed)`
with:

```rust
let stem = item.input_path.file_stem().unwrap_or_default().to_string_lossy();
let body = match extract_url(&content) {
    Some(url) if !stem.is_empty() => to_markdown(&stem, url),
    // A3 fallback: no URL, empty URL value, or empty stem → raw text,
    // normalized to exactly one trailing newline.
    _ => format!("{}\n", trimmed),
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
- **`extract_url` borrows (`Option<&str>`), not `Option<String>`.** The value is only
  read (formatted into the link or ignored for the fallback), so there is no reason to
  allocate. Per the Codex review.

### Deliberately out of scope (reviewed, declined)

The Codex review raised three further ideas that we are **not** doing, to keep this change
minimal and honest to the "local, no resolution" goal:

- **Section-awareness** (only accept `URL=` under `[InternetShortcut]`). Real `.url` files
  put the key in that section; scoping adds parser state for a case that does not occur in
  practice. YAGNI.
- **Markdown-escaping** of `[`, `]`, `)` in the stem/URL. Escaping characters inside a URL
  is arguably wrong (it changes the link target), and stems are ordinary filenames. Not
  worth the complexity or the risk of mangling valid URLs.
- **BOM/encoding handling and stale-`.md` cleanup.** No evidence the input files carry
  BOMs; cleaning stale outputs from prior runs is a separate feature, not part of fixing
  conversion.

## Testing

Add a `#[cfg(test)] mod tests` to `src/main.rs`. The pure functions make this clean —
no temp dirs, no tokio.

`extract_url` (returns `Option<&str>`):
- standard `[InternetShortcut]\nURL=https://github.com` → `Some("https://github.com")`
- URL with query string containing `=` → full value preserved
- lowercase `url=` → matched (case-insensitive)
- `URL=` empty value → `None` (drives A3 fallback)
- no URL line (`[InternetShortcut]` only) → `None`
- URL not on the first line (icon/other keys before it) → found
- **first-key-wins:** an empty `URL=` line *followed by* `URL=https://real.com` →
  `None` (the scan stops at the first `URL` key; the later line is never read). This is
  the regression the review caught — assert it explicitly.

`to_markdown`:
- `to_markdown("github", "https://github.com")` == `"[github](https://github.com)\n"`

The empty-stem fallback is enforced at the call site (`Some(url) if !stem.is_empty()`),
not inside `extract_url`, so it is covered by the end-to-end verification below rather than
a pure unit test.

Existing `test_data/*.md` fixtures (`github.md`, `google.md`) currently hold the *old*
raw-copy output. They will be **updated** to the new expected link output as part of
implementation, so they document real behavior. Each ends with a single trailing newline
(the `\n` from `to_markdown`):

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
