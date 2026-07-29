use std::fmt::Write as _;

use crate::diagnostic::{Diagnostic, Severity};
use crate::source_map::SourceMap;

/// Renders a [`Diagnostic`] as rustc-style human-readable text: severity and
/// message, a `-->` location line per label, the highlighted source line(s),
/// then hints and suggestions.
///
/// Multi-line spans are underlined only on their first line in this MVP
/// renderer — see `docs/architecture/crates.md` (`diagnostics`) for this as
/// a documented simplification, not an oversight.
pub fn render(diag: &Diagnostic, map: &SourceMap) -> String {
    let mut out = String::new();

    let severity_word = match diag.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Note => "note",
    };
    let _ = writeln!(out, "{severity_word}: {}", diag.message);

    for label in &diag.labels {
        let start = map.line_col(label.span.file, label.span.start);
        let file_name = map.file_name(label.span.file).display();
        let _ = writeln!(out, "  --> {file_name}:{}:{}", start.line, start.column);

        let line_no = start.line;
        let line_text = map.line_text(label.span.file, (line_no - 1) as usize);
        let gutter_width = line_no.to_string().len();

        let _ = writeln!(out, "{:gutter_width$} |", "");
        let _ = writeln!(out, "{line_no:gutter_width$} | {line_text}");

        let end_col = if label.span.is_empty() {
            start.column + 1
        } else {
            let end = map.line_col(label.span.file, label.span.end);
            if end.line == start.line {
                end.column
            } else {
                (line_text.chars().count() as u32) + 1
            }
        };
        let underline_len = end_col.saturating_sub(start.column).max(1);
        let padding = " ".repeat(start.column as usize - 1);
        let underline = "^".repeat(underline_len as usize);
        let _ = writeln!(
            out,
            "{:gutter_width$} | {padding}{underline} {}",
            "", label.message
        );
    }

    if !diag.hints.is_empty() || !diag.suggestions.is_empty() {
        let _ = writeln!(out, "   |");
        for hint in &diag.hints {
            let _ = writeln!(out, "   = hint: {hint}");
        }
        for suggestion in &diag.suggestions {
            let _ = writeln!(
                out,
                "   = suggestion: {} (`{}`)",
                suggestion.message, suggestion.replacement
            );
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Diagnostic;
    use crate::source_map::Span;

    #[test]
    fn renders_error_with_label_and_hint() {
        let mut map = SourceMap::new();
        let file = map.add_file("a.nr", "let 4a = 1;\n");
        let span = Span::new(file, 4, 6); // "4a"
        let diag = Diagnostic::error("invalid identifier")
            .with_label(span, "identifiers cannot start with a digit")
            .with_hint("rename this binding");

        let rendered = render(&diag, &map);
        assert!(rendered.starts_with("error: invalid identifier\n"));
        assert!(rendered.contains("a.nr:1:5"));
        assert!(rendered.contains("let 4a = 1;"));
        assert!(rendered.contains("^^ identifiers cannot start with a digit"));
        assert!(rendered.contains("hint: rename this binding"));
    }

    #[test]
    fn renders_warning_severity_word() {
        let mut map = SourceMap::new();
        map.add_file("a.nr", "let a = 1;\n");
        let diag = Diagnostic::warning("unused variable");
        let rendered = render(&diag, &map);
        assert!(rendered.starts_with("warning: unused variable\n"));
    }
}
