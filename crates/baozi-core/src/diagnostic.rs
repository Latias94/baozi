use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiagnosticCode(pub &'static str);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceLocation {
    pub byte_offset: Option<u64>,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

impl SourceLocation {
    pub const fn byte(byte_offset: u64) -> Self {
        Self {
            byte_offset: Some(byte_offset),
            line: None,
            column: None,
        }
    }

    pub const fn line_column(line: u32, column: u32) -> Self {
        Self {
            byte_offset: None,
            line: Some(line),
            column: Some(column),
        }
    }
}

impl fmt::Display for SourceLocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.line, self.column, self.byte_offset) {
            (Some(line), Some(column), Some(byte)) => {
                write!(formatter, "line {line}, column {column}, byte {byte}")
            }
            (Some(line), Some(column), None) => write!(formatter, "line {line}, column {column}"),
            (_, _, Some(byte)) => write!(formatter, "byte {byte}"),
            _ => write!(formatter, "unknown location"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub code: DiagnosticCode,
    pub source: Option<String>,
    pub location: Option<SourceLocation>,
    pub message: String,
}

impl Diagnostic {
    pub fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: DiagnosticSeverity::Warning,
            code: DiagnosticCode(code),
            source: None,
            location: None,
            message: message.into(),
        }
    }
}
