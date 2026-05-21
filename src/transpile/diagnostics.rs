use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Error => write!(f, "error"),
            Self::Warning => write!(f, "warning"),
            Self::Note => write!(f, "note"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub source_id: Option<String>,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            source_id: None,
        }
    }

    pub fn with_source(mut self, source_id: impl Into<String>) -> Self {
        self.source_id = Some(source_id.into());
        self
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(id) = &self.source_id {
            write!(f, "{id}:{}-{}", self.start, self.end)
        } else {
            write!(f, "{}-{}", self.start, self.end)
        }
    }
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub span: Option<Span>,
}

impl Diagnostic {
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            span: None,
        }
    }

    pub fn error_at(span: Span, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            span: Some(span),
        }
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            span: None,
        }
    }

    pub fn warning_at(span: Span, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            span: Some(span),
        }
    }

    pub fn note(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Note,
            message: message.into(),
            span: None,
        }
    }

    pub fn note_at(span: Span, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Note,
            message: message.into(),
            span: Some(span),
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(span) = &self.span {
            write!(f, "[{}] {}: {}", span, self.severity, self.message)
        } else {
            write!(f, "{}: {}", self.severity, self.message)
        }
    }
}

#[derive(Debug, Default)]
pub struct Diagnostics {
    pub diagnostics: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, diag: Diagnostic) {
        self.diagnostics.push(diag);
    }

    pub fn error(&mut self, msg: impl Into<String>) {
        self.push(Diagnostic::error(msg));
    }

    pub fn error_at(&mut self, span: Span, msg: impl Into<String>) {
        self.push(Diagnostic::error_at(span, msg));
    }

    pub fn warning(&mut self, msg: impl Into<String>) {
        self.push(Diagnostic::warning(msg));
    }

    pub fn warning_at(&mut self, span: Span, msg: impl Into<String>) {
        self.push(Diagnostic::warning_at(span, msg));
    }

    pub fn note(&mut self, msg: impl Into<String>) {
        self.push(Diagnostic::note(msg));
    }

    pub fn note_at(&mut self, span: Span, msg: impl Into<String>) {
        self.push(Diagnostic::note_at(span, msg));
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }

    pub fn diagnostics(self) -> impl Iterator<Item = Diagnostic> {
        self.diagnostics.into_iter()
    }
}

impl Extend<Diagnostic> for Diagnostics {
    fn extend<T: IntoIterator<Item = Diagnostic>>(&mut self, iter: T) {
        self.diagnostics.extend(iter);
    }
}

impl FromIterator<Diagnostic> for Diagnostics {
    fn from_iter<T: IntoIterator<Item = Diagnostic>>(iter: T) -> Self {
        let mut diags = Self::default();
        diags.extend(iter);
        diags
    }
}
