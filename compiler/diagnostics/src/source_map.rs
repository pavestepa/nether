use std::path::{Path, PathBuf};

/// Identifies one loaded source file within a [`SourceMap`].
///
/// Stable for the lifetime of the `SourceMap` that produced it: indices are
/// never reused, so a `FileId` obtained early in a compilation stays valid
/// (and meaningful) throughout every later stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(pub(crate) u32);

/// A byte-offset range within one source file.
///
/// This is the only source-location representation any compiler crate
/// upstream of `typecheck` should carry through its own logic — resolving a
/// `Span` to a human-readable line/column is [`SourceMap`]'s job, done only
/// at render time (see the crate-level docs for why this split matters).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(file: FileId, start: u32, end: u32) -> Self {
        debug_assert!(start <= end, "span start must not be after its end");
        Span { file, start, end }
    }

    /// The smallest span covering both `self` and `other`.
    ///
    /// Used to build a span for a composite AST node (e.g. a binary
    /// expression) from its parts' spans.
    ///
    /// # Panics
    /// Panics (in debug builds) if the two spans are not in the same file —
    /// combining spans across files is never meaningful.
    pub fn to(self, other: Span) -> Span {
        debug_assert_eq!(
            self.file, other.file,
            "cannot merge spans from different files"
        );
        Span::new(
            self.file,
            self.start.min(other.start),
            self.end.max(other.end),
        )
    }

    pub fn len(self) -> u32 {
        self.end - self.start
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// 1-based line and column, for human-facing rendering only.
///
/// Internal logic must never compare or store `LineCol` in place of a
/// `Span` — it exists purely to feed [`render`](crate::render).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCol {
    pub line: u32,
    pub column: u32,
}

struct FileEntry {
    name: PathBuf,
    source: String,
    /// Byte offset of the first character of each line; `line_starts[0] == 0`.
    line_starts: Vec<u32>,
}

/// Owns every source file loaded during one compilation and maps byte
/// offsets back to file/line/column for diagnostic rendering.
///
/// See the crate-level docs for `SourceMap`'s role in the overall
/// diagnostics architecture.
#[derive(Default)]
pub struct SourceMap {
    files: Vec<FileEntry>,
}

impl SourceMap {
    pub fn new() -> Self {
        SourceMap { files: Vec::new() }
    }

    /// Registers a source file and returns the [`FileId`] to build [`Span`]s
    /// against.
    pub fn add_file(&mut self, name: impl Into<PathBuf>, source: impl Into<String>) -> FileId {
        let source = source.into();
        let line_starts = compute_line_starts(&source);
        let id = FileId(self.files.len() as u32);
        self.files.push(FileEntry {
            name: name.into(),
            source,
            line_starts,
        });
        id
    }

    pub fn source(&self, file: FileId) -> &str {
        &self.files[file.0 as usize].source
    }

    pub fn file_name(&self, file: FileId) -> &Path {
        &self.files[file.0 as usize].name
    }

    /// The text a `Span` covers. Panics if `span` is out of bounds for its
    /// file — every `Span` handed to a `SourceMap` is expected to have been
    /// built from that same map's source text.
    pub fn snippet(&self, span: Span) -> &str {
        &self.source(span.file)[span.start as usize..span.end as usize]
    }

    /// Converts a byte offset into a 1-based line/column.
    pub fn line_col(&self, file: FileId, offset: u32) -> LineCol {
        let entry = &self.files[file.0 as usize];
        let line_idx = match entry.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let line_start = entry.line_starts[line_idx];
        LineCol {
            line: (line_idx + 1) as u32,
            column: offset - line_start + 1,
        }
    }

    /// The full text of the (0-based) line index, without its trailing
    /// newline.
    pub fn line_text(&self, file: FileId, line_idx0: usize) -> &str {
        let entry = &self.files[file.0 as usize];
        let start = entry.line_starts[line_idx0] as usize;
        let end = entry
            .line_starts
            .get(line_idx0 + 1)
            .map(|&s| s as usize - 1) // exclude the '\n'
            .unwrap_or(entry.source.len());
        entry.source[start..end].trim_end_matches('\r')
    }
}

fn compute_line_starts(source: &str) -> Vec<u32> {
    let mut starts = vec![0u32];
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            starts.push((i + 1) as u32);
        }
    }
    starts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_first_line() {
        let mut map = SourceMap::new();
        let f = map.add_file("a.nr", "let a = 1;\nlet b = 2;\n");
        assert_eq!(map.line_col(f, 0), LineCol { line: 1, column: 1 });
        assert_eq!(map.line_col(f, 4), LineCol { line: 1, column: 5 });
    }

    #[test]
    fn line_col_second_line() {
        let mut map = SourceMap::new();
        let f = map.add_file("a.nr", "let a = 1;\nlet b = 2;\n");
        let second_line_start = 11; // byte offset of 'l' in "let b"
        assert_eq!(
            map.line_col(f, second_line_start),
            LineCol { line: 2, column: 1 }
        );
    }

    #[test]
    fn line_text_strips_newline() {
        let mut map = SourceMap::new();
        let f = map.add_file("a.nr", "first\nsecond\nthird");
        assert_eq!(map.line_text(f, 0), "first");
        assert_eq!(map.line_text(f, 1), "second");
        assert_eq!(map.line_text(f, 2), "third");
    }

    #[test]
    fn snippet_extracts_span_text() {
        let mut map = SourceMap::new();
        let f = map.add_file("a.nr", "let a = 1;");
        let span = Span::new(f, 4, 5);
        assert_eq!(map.snippet(span), "a");
    }

    #[test]
    fn span_to_merges_ranges() {
        let mut map = SourceMap::new();
        let f = map.add_file("a.nr", "let a = 1;");
        let a = Span::new(f, 0, 3);
        let b = Span::new(f, 4, 5);
        let merged = a.to(b);
        assert_eq!(merged, Span::new(f, 0, 5));
    }
}
