use crate::source_map::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

/// A span-anchored note attached to a [`Diagnostic`] — e.g. "expected `;`
/// here" pointing at the exact byte range that's wrong.
#[derive(Debug, Clone)]
pub struct Label {
    pub span: Span,
    pub message: String,
}

/// A machine-applicable source rewrite offered alongside a diagnostic.
#[derive(Debug, Clone)]
pub struct Suggestion {
    pub span: Span,
    pub replacement: String,
    pub message: String,
}

/// One compiler diagnostic: a severity, a primary message, and any number
/// of [`Label`]s, free-text hints, and [`Suggestion`]s.
///
/// Built with the `with_*` methods rather than struct-literal syntax so
/// call sites read as a short chain (`Diagnostic::error(..).with_label(..)`)
/// matching the shape used throughout `lexer`/`parser`.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub labels: Vec<Label>,
    pub hints: Vec<String>,
    pub suggestions: Vec<Suggestion>,
}

impl Diagnostic {
    fn new(severity: Severity, message: impl Into<String>) -> Self {
        Diagnostic {
            severity,
            message: message.into(),
            labels: Vec::new(),
            hints: Vec::new(),
            suggestions: Vec::new(),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::new(Severity::Error, message)
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, message)
    }

    pub fn note(message: impl Into<String>) -> Self {
        Self::new(Severity::Note, message)
    }

    pub fn with_label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(Label { span, message: message.into() });
        self
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hints.push(hint.into());
        self
    }

    pub fn with_suggestion(
        mut self,
        span: Span,
        replacement: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        self.suggestions.push(Suggestion {
            span,
            replacement: replacement.into(),
            message: message.into(),
        });
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}
