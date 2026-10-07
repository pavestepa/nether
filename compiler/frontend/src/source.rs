#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub source: SourceId,
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn join(self, other: Self) -> Self {
        assert_eq!(self.source, other.source);
        Self {
            source: self.source,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
    pub span: Span,
}

impl Diagnostic {
    pub fn new(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        Self {
            code,
            message: message.into(),
            span,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Source {
    pub name: String,
    pub text: String,
    line_starts: Vec<usize>,
}

impl Source {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Self {
        let text = text.into();
        let mut starts = vec![0];
        let mut chars = text.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == '\r' {
                let end = if chars.peek().is_some_and(|(_, next)| *next == '\n') {
                    chars.next();
                    i + 2
                } else {
                    i + 1
                };
                starts.push(end);
            } else if c == '\n' {
                starts.push(i + 1);
            }
        }
        Self {
            name: name.into(),
            text,
            line_starts: starts,
        }
    }

    pub fn location(&self, byte: usize) -> Option<(usize, usize)> {
        if byte > self.text.len() || !self.text.is_char_boundary(byte) {
            return None;
        }
        let line = self.line_starts.partition_point(|start| *start <= byte) - 1;
        Some((
            line + 1,
            self.text[self.line_starts[line]..byte].chars().count() + 1,
        ))
    }

    pub fn render(&self, diagnostic: &Diagnostic) -> String {
        let (line, column) = self.location(diagnostic.span.start).unwrap_or((1, 1));
        format!(
            "{}:{line}:{column}: error[{}]: {}",
            self.name, diagnostic.code, diagnostic.message
        )
    }
}
