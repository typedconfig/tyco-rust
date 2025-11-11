use std::{fmt, io, path::PathBuf};

/// Describes a precise source location for a parser error.
#[derive(Clone, Debug)]
pub struct SourceSpan {
    pub path: Option<PathBuf>,
    pub line: usize,
    pub column: usize,
    pub line_text: String,
}

impl SourceSpan {
    pub fn display(&self) -> String {
        let location = match &self.path {
            Some(path) => format!(
                "File \"{}\", line {}, column {}:",
                path.display(),
                self.line,
                self.column
            ),
            None => format!("Line {}, column {}:", self.line, self.column),
        };

        let mut pointer = String::new();
        let mut visual_col = 0usize;
        for (idx, ch) in self.line_text.chars().enumerate() {
            if idx >= self.column.saturating_sub(1) {
                break;
            }
            if ch == '\t' {
                let next_tab = ((visual_col / 8) + 1) * 8;
                pointer.push_str(&" ".repeat(next_tab - visual_col));
                visual_col = next_tab;
            } else {
                pointer.push(' ');
                visual_col += 1;
            }
        }
        pointer.push('^');
        format!("{location}\n{}\n{pointer}", self.line_text)
    }
}

/// Shared error type for the Tyco parser.
#[derive(Debug)]
pub enum TycoError {
    Io(io::Error),
    Parse { message: String, span: Option<SourceSpan> },
    UnknownStruct(String),
    Reference(String),
}

impl TycoError {
    pub fn parse(msg: impl Into<String>) -> Self {
        TycoError::Parse {
            message: msg.into(),
            span: None,
        }
    }

    pub fn parse_with_span(msg: impl Into<String>, span: SourceSpan) -> Self {
        TycoError::Parse {
            message: msg.into(),
            span: Some(span),
        }
    }

    pub fn with_span(self, span: SourceSpan) -> Self {
        match self {
            TycoError::Parse { message, .. } => TycoError::Parse {
                message,
                span: Some(span),
            },
            other => other,
        }
    }
}

impl fmt::Display for TycoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TycoError::Io(err) => write!(f, "I/O error: {err}"),
            TycoError::Parse { message, span } => {
                write!(f, "Parse error: {message}")?;
                if let Some(span) = span {
                    write!(f, "\n{}", span.display())?;
                }
                Ok(())
            }
            TycoError::UnknownStruct(name) => write!(f, "Unknown struct '{name}'"),
            TycoError::Reference(message) => write!(f, "Reference error: {message}"),
        }
    }
}

impl std::error::Error for TycoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TycoError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for TycoError {
    fn from(value: io::Error) -> Self {
        TycoError::Io(value)
    }
}
