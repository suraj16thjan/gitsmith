use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    FileHeader,
    HunkHeader,
    Context,
    Change,
    Removed,
    Added,
}

#[derive(Debug, Clone)]
pub struct DiffRow {
    pub kind: RowKind,
    pub left: String,
    pub right: String,
    pub lang: String,
    /// Old-file (left) line number; `None` for added lines and header rows.
    pub lnum_left: Option<u32>,
    /// New-file (right) line number; `None` for removed lines and header rows.
    pub lnum_right: Option<u32>,
}

#[derive(Debug, Default)]
pub struct ParsedDiff {
    pub rows: Vec<DiffRow>,
    pub files: usize,
}

impl ParsedDiff {
    /// Each file in the diff as (path, the row its header sits on), in order.
    /// Drives the file list beside the diff and the `[` / `]` jumps.
    pub fn file_positions(&self) -> Vec<(&str, usize)> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kind == RowKind::FileHeader)
            .map(|(i, r)| (r.left.as_str(), i))
            .collect()
    }

    /// Index into `file_positions` of the file that `row` belongs to.
    pub fn file_at(&self, row: usize) -> usize {
        let files = self.file_positions();
        files.iter().rposition(|(_, at)| *at <= row).unwrap_or(0)
    }
}

fn ext_of(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string()
}

fn is_meta(line: &str) -> bool {
    const PREFIXES: [&str; 9] = [
        "index ", "new file", "deleted file", "similarity ", "rename ",
        "old mode", "new mode", "\\ No newline", "Binary files",
    ];
    PREFIXES.iter().any(|p| line.starts_with(p))
}

/// Parse the starting line numbers from a hunk header `@@ -old[,n] +new[,m] @@`.
/// Returns `(old_start, new_start)`; `None` if the header is malformed.
fn hunk_starts(line: &str) -> Option<(u32, u32)> {
    let body = line.strip_prefix("@@")?.split("@@").next()?;
    let mut old = None;
    let mut new = None;
    for tok in body.split_whitespace() {
        if let Some(o) = tok.strip_prefix('-') {
            old = o.split(',').next().and_then(|n| n.parse().ok());
        } else if let Some(n) = tok.strip_prefix('+') {
            new = n.split(',').next().and_then(|n| n.parse().ok());
        }
    }
    Some((old?, new?))
}

pub fn parse(unified: &str) -> ParsedDiff {
    let lines: Vec<&str> = unified.lines().collect();
    let mut rows = Vec::new();
    let mut files = 0;
    let mut lang = String::new();
    let mut i = 0;
    let mut in_hunk = false;
    let mut file_headered = false; // did the current file already get a header row?
    // Line-number counters for the current hunk, set from each `@@` header.
    let mut old_ln = 0u32;
    let mut new_ln = 0u32;

    let row = |kind, left: &str, right: &str, lang: &str, ll, rl| DiffRow {
        kind,
        left: left.to_string(),
        right: right.to_string(),
        lang: lang.to_string(),
        lnum_left: ll,
        lnum_right: rl,
    };

    while i < lines.len() {
        let line = lines[i];

        if let Some(rest) = line.strip_prefix("diff --git ") {
            files += 1;
            let newpath = rest.split_whitespace().nth(1).unwrap_or("");
            let path = newpath.strip_prefix("b/").unwrap_or(newpath);
            lang = ext_of(path);
            rows.push(row(RowKind::FileHeader, path, "", &lang, None, None));
            file_headered = true;
            in_hunk = false;
            i += 1;
        } else if line.starts_with("@@") {
            if let Some((o, n)) = hunk_starts(line) {
                old_ln = o;
                new_ln = n;
            }
            rows.push(row(RowKind::HunkHeader, line, "", &lang, None, None));
            in_hunk = true;
            i += 1;
        } else if is_meta(line) {
            i += 1;
        } else if !in_hunk && line.starts_with("--- ") {
            // old-file marker before the first hunk; the following `+++` has the path
            i += 1;
        } else if !in_hunk && line.starts_with("+++ ") {
            // New-file header line. When the diff has no `diff --git` line (e.g. some
            // `glab mr diff` output), derive the file header from here so the filename
            // still shows. Inside a hunk, `+++ …` is added content, not a header.
            if !file_headered {
                let p = line.strip_prefix("+++ ").unwrap_or("");
                let p = p.split('\t').next().unwrap_or(p);
                let path = p.strip_prefix("b/").unwrap_or(p);
                files += 1;
                lang = ext_of(path);
                rows.push(row(RowKind::FileHeader, path, "", &lang, None, None));
                file_headered = true;
            }
            i += 1;
        } else if let Some(text) = line.strip_prefix(' ') {
            rows.push(row(RowKind::Context, text, text, &lang, Some(old_ln), Some(new_ln)));
            old_ln += 1;
            new_ln += 1;
            i += 1;
        } else if line.starts_with('-') || line.starts_with('+') {
            // A run of removals then additions. removed[k] is old line `old_ln + k`;
            // added[k] is new line `new_ln + k` — so a paired Change carries both.
            let (run_old, run_new) = (old_ln, new_ln);
            let mut removed = Vec::new();
            while i < lines.len() && lines[i].starts_with('-') {
                removed.push(&lines[i][1..]);
                i += 1;
            }
            let mut added = Vec::new();
            while i < lines.len() && lines[i].starts_with('+') {
                added.push(&lines[i][1..]);
                i += 1;
            }
            let n = removed.len().max(added.len());
            for k in 0..n {
                let (ll, rl) = (run_old + k as u32, run_new + k as u32);
                match (removed.get(k), added.get(k)) {
                    (Some(l), Some(r)) => rows.push(row(RowKind::Change, l, r, &lang, Some(ll), Some(rl))),
                    (Some(l), None) => rows.push(row(RowKind::Removed, l, "", &lang, Some(ll), None)),
                    (None, Some(r)) => rows.push(row(RowKind::Added, "", r, &lang, None, Some(rl))),
                    (None, None) => {}
                }
            }
            old_ln += removed.len() as u32;
            new_ln += added.len() as u32;
        } else {
            i += 1;
        }
    }

    ParsedDiff { rows, files }
}

#[cfg(test)]
mod file_nav_tests {
    use super::*;

    fn two_files() -> ParsedDiff {
        let mut d = String::new();
        for f in ["src/a.rs", "src/b.rs"] {
            d.push_str(&format!("diff --git a/{f} b/{f}\n@@ -1,2 +1,2 @@\n one\n two\n"));
        }
        parse(&d)
    }

    #[test]
    fn file_positions_and_lookup() {
        let d = two_files();
        let files = d.file_positions();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].0, "src/a.rs");
        assert_eq!(files[1].0, "src/b.rs");
        // rows before the second header belong to the first file
        assert_eq!(d.file_at(0), 0);
        assert_eq!(d.file_at(files[1].1 - 1), 0);
        // …and from its header on, to the second
        assert_eq!(d.file_at(files[1].1), 1);
        assert_eq!(d.file_at(d.rows.len() - 1), 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
diff --git a/src/main.rs b/src/main.rs
index abc..def 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,3 @@
 fn main() {
-    println!(\"old\");
+    println!(\"new\");
 }
";

    #[test]
    fn parses_header_hunk_context_and_change() {
        let d = parse(SAMPLE);
        assert_eq!(d.files, 1);
        assert_eq!(d.rows[0].kind, RowKind::FileHeader);
        assert_eq!(d.rows[0].left, "src/main.rs");
        assert_eq!(d.rows[0].lang, "rs");
        assert_eq!(d.rows[1].kind, RowKind::HunkHeader);
        assert_eq!(d.rows[2].kind, RowKind::Context);
        assert_eq!(d.rows[2].left, "fn main() {");
        assert_eq!(d.rows[2].right, "fn main() {");
        assert_eq!(d.rows[3].kind, RowKind::Change);
        assert_eq!(d.rows[3].left, "    println!(\"old\");");
        assert_eq!(d.rows[3].right, "    println!(\"new\");");
        assert_eq!(d.rows[4].kind, RowKind::Context);
    }

    #[test]
    fn uneven_runs_pad_with_removed_added() {
        let d = parse("@@ -1,2 +1,1 @@\n-a\n-b\n+c\n");
        // run: 2 removed, 1 added → Change(a,c) then Removed(b)
        assert_eq!(d.rows[1].kind, RowKind::Change);
        assert_eq!((d.rows[1].left.as_str(), d.rows[1].right.as_str()), ("a", "c"));
        assert_eq!(d.rows[2].kind, RowKind::Removed);
        assert_eq!((d.rows[2].left.as_str(), d.rows[2].right.as_str()), ("b", ""));
    }

    #[test]
    fn added_only_run() {
        let d = parse("@@ -0,0 +1,1 @@\n+x\n");
        assert_eq!(d.rows[1].kind, RowKind::Added);
        assert_eq!((d.rows[1].left.as_str(), d.rows[1].right.as_str()), ("", "x"));
    }

    #[test]
    fn counts_multiple_files() {
        let d = parse("diff --git a/x.py b/x.py\n@@ -1 +1 @@\n-p\n+q\ndiff --git a/y.js b/y.js\n@@ -1 +1 @@\n-r\n+s\n");
        assert_eq!(d.files, 2);
        assert_eq!(d.rows.iter().filter(|r| r.kind == RowKind::FileHeader).count(), 2);
    }

    #[test]
    fn empty_input() {
        let d = parse("");
        assert_eq!(d.files, 0);
        assert_eq!(d.rows.len(), 0);
    }

    #[test]
    fn only_removed_lines() {
        let d = parse("@@ -1,2 +1,0 @@\n-a\n-b\n");
        assert_eq!(d.rows[0].kind, RowKind::HunkHeader);
        assert_eq!(d.rows[1].kind, RowKind::Removed);
        assert_eq!(d.rows[1].left, "a");
        assert_eq!(d.rows[1].right, "");
        assert_eq!(d.rows[2].kind, RowKind::Removed);
    }

    #[test]
    fn only_added_lines() {
        let d = parse("@@ -0,0 +1,2 @@\n+a\n+b\n");
        assert_eq!(d.rows[1].kind, RowKind::Added);
        assert_eq!(d.rows[1].left, "");
        assert_eq!(d.rows[1].right, "a");
    }

    #[test]
    fn assigns_line_numbers_context_and_change() {
        let d = parse(SAMPLE); // @@ -1,3 +1,3 @@
        assert_eq!((d.rows[2].lnum_left, d.rows[2].lnum_right), (Some(1), Some(1))); // context
        assert_eq!((d.rows[3].lnum_left, d.rows[3].lnum_right), (Some(2), Some(2))); // change
        assert_eq!((d.rows[4].lnum_left, d.rows[4].lnum_right), (Some(3), Some(3))); // context
    }

    #[test]
    fn line_numbers_track_uneven_run() {
        // @@ -5,1 +5,2 @@  context, then remove 1 / add 2
        let d = parse("@@ -5,1 +5,2 @@\n ctx\n-gone\n+new1\n+new2\n");
        assert_eq!((d.rows[1].lnum_left, d.rows[1].lnum_right), (Some(5), Some(5))); // ctx
        // run starts at old 6 / new 6: Change(gone,new1) then Added(new2)
        assert_eq!(d.rows[2].kind, RowKind::Change);
        assert_eq!((d.rows[2].lnum_left, d.rows[2].lnum_right), (Some(6), Some(6)));
        assert_eq!(d.rows[3].kind, RowKind::Added);
        assert_eq!((d.rows[3].lnum_left, d.rows[3].lnum_right), (None, Some(7)));
    }

    #[test]
    fn header_rows_have_no_line_numbers() {
        let d = parse(SAMPLE);
        assert_eq!((d.rows[0].lnum_left, d.rows[0].lnum_right), (None, None)); // file header
        assert_eq!((d.rows[1].lnum_left, d.rows[1].lnum_right), (None, None)); // hunk header
    }

    #[test]
    fn file_header_derived_from_plus_line_without_diff_git() {
        // Some `glab mr diff` output omits the `diff --git` line.
        let d = parse("--- a/src/app.js\n+++ b/src/app.js\n@@ -1 +1 @@\n-a\n+b\n");
        assert_eq!(d.files, 1);
        assert_eq!(d.rows[0].kind, RowKind::FileHeader);
        assert_eq!(d.rows[0].left, "src/app.js");
        assert_eq!(d.rows[0].lang, "js");
    }

    #[test]
    fn dashdash_content_inside_hunk_is_not_metadata() {
        let d = parse("@@ -1,1 +1,1 @@\n--- old comment\n+++ new comment\n");
        assert_eq!(d.rows[1].kind, RowKind::Change);
        assert_eq!(d.rows[1].left, "-- old comment");
        assert_eq!(d.rows[1].right, "++ new comment");
    }
}
