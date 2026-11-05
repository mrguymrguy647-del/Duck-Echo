//! Agent-friendly diagnostics.
//!
//! Every problem is an [`Issue`] with an exact field path, a stable machine
//! readable `code`, and, where it applies, the allowed range or options and a
//! "did you mean" suggestion. A [`Report`] collects all issues at once so an
//! agent can fix everything in one edit.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Stable machine-readable error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    /// The file is not valid TOML or JSON.
    Syntax,
    /// A field name that does not exist on this object.
    UnknownField,
    /// An enum value (for example a `source` or `type`) that does not exist.
    UnknownVariant,
    /// A required field is missing.
    MissingField,
    /// A value has the wrong type (for example a string where a number goes).
    TypeMismatch,
    /// A number outside its allowed range.
    OutOfRange,
    /// The `version` field is not supported by this build.
    UnsupportedVersion,
    /// The recipe cannot render as written (for example it has no layers).
    Invalid,
    /// Anything else that stops the recipe from loading.
    Other,
    /// A field that is set but has no effect in this context.
    Unused,
    /// Something that renders but is probably not what was intended.
    Suspicious,
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Code::Syntax => "syntax",
            Code::UnknownField => "unknown_field",
            Code::UnknownVariant => "unknown_variant",
            Code::MissingField => "missing_field",
            Code::TypeMismatch => "type_mismatch",
            Code::OutOfRange => "out_of_range",
            Code::UnsupportedVersion => "unsupported_version",
            Code::Invalid => "invalid",
            Code::Other => "other",
            Code::Unused => "unused",
            Code::Suspicious => "suspicious",
        };
        f.write_str(s)
    }
}

/// What a field is allowed to be.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Allowed {
    /// A closed numeric range (inclusive).
    Range {
        /// Smallest allowed value.
        min: f64,
        /// Largest allowed value.
        max: f64,
    },
    /// A closed set of options.
    OneOf {
        /// The valid options.
        options: Vec<String>,
    },
}

/// Severity of an issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// The recipe cannot be used.
    Error,
    /// The recipe works but something looks off.
    Warning,
}

/// One diagnostic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Issue {
    /// Error or warning.
    pub severity: Severity,
    /// Stable machine-readable code.
    pub code: Code,
    /// Exact field path, for example `layers[1].pitch.start`. Empty for the
    /// root document.
    pub path: String,
    /// Human-readable explanation.
    pub message: String,
    /// The offending value, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub got: Option<serde_json::Value>,
    /// What would have been accepted, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed: Option<Allowed>,
    /// A likely intended value or field name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    /// 1-based source line, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// 1-based source column, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
}

impl Issue {
    /// A new error with just a code, path and message.
    pub fn error(code: Code, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code,
            path: path.into(),
            message: message.into(),
            got: None,
            allowed: None,
            suggestion: None,
            line: None,
            column: None,
        }
    }

    /// A new warning with just a code, path and message.
    pub fn warning(code: Code, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            ..Self::error(code, path, message)
        }
    }

    /// Attach the offending value.
    pub fn got(mut self, value: impl Into<serde_json::Value>) -> Self {
        self.got = Some(value.into());
        self
    }

    /// Attach an allowed range.
    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.allowed = Some(Allowed::Range { min, max });
        self
    }

    /// Attach a set of allowed options.
    pub fn one_of<I, S>(mut self, options: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.allowed = Some(Allowed::OneOf {
            options: options.into_iter().map(Into::into).collect(),
        });
        self
    }

    /// Attach a suggestion.
    pub fn suggest(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    /// Attach a source position (1-based).
    pub fn at(mut self, line: usize, column: usize) -> Self {
        self.line = Some(line);
        self.column = Some(column);
        self
    }
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{sev}[{}]", self.code)?;
        if let (Some(line), Some(col)) = (self.line, self.column) {
            write!(f, " line {line}:{col}")?;
        }
        if self.path.is_empty() {
            write!(f, ": {}", self.message)?;
        } else {
            write!(f, " {}: {}", self.path, self.message)?;
        }
        if let Some(got) = &self.got {
            write!(f, " (got {got})")?;
        }
        match &self.allowed {
            Some(Allowed::Range { min, max }) => write!(f, " [allowed {min}..={max}]")?,
            Some(Allowed::OneOf { options }) => write!(f, " [allowed: {}]", options.join(", "))?,
            None => {}
        }
        if let Some(s) = &self.suggestion {
            write!(f, " Did you mean `{s}`?")?;
        }
        Ok(())
    }
}

/// All diagnostics for one recipe. `ok` is true when there are no errors;
/// warnings never block.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// True when there are no errors.
    pub ok: bool,
    /// Blocking problems.
    pub errors: Vec<Issue>,
    /// Non-blocking observations.
    pub warnings: Vec<Issue>,
}

impl Report {
    /// An empty, passing report.
    pub fn new() -> Self {
        Self {
            ok: true,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Add an issue, routing it by severity.
    pub fn push(&mut self, issue: Issue) {
        match issue.severity {
            Severity::Error => {
                self.errors.push(issue);
                self.ok = false;
            }
            Severity::Warning => self.warnings.push(issue),
        }
    }

    /// Merge another report into this one.
    pub fn extend(&mut self, other: Report) {
        for i in other.errors.into_iter().chain(other.warnings) {
            self.push(i);
        }
    }

    /// True when there are no errors.
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// JSON form, pretty printed.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("report serializes")
    }

    /// Build a report from a single error.
    pub fn from_error(issue: Issue) -> Self {
        let mut r = Self::new();
        r.push(issue);
        r
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for e in &self.errors {
            writeln!(f, "{e}")?;
        }
        for w in &self.warnings {
            writeln!(f, "{w}")?;
        }
        if self.errors.is_empty() {
            if self.warnings.is_empty() {
                writeln!(f, "ok: no problems found")
            } else {
                writeln!(f, "ok with {} warning(s)", self.warnings.len())
            }
        } else {
            writeln!(
                f,
                "{} error(s), {} warning(s)",
                self.errors.len(),
                self.warnings.len()
            )
        }
    }
}

impl std::error::Error for Report {}
