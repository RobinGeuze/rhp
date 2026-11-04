//! The interpreter's unwinding type, at the leaf of the crate graph so that
//! `rhp-semantics` (operators), `rhp-builtins`, the compiler and the VM can
//! all return `Result<_, Thrown>` without a dependency cycle.

/// Unwinds the interpreter: a real PHP `Throwable`, an engine refusal for an
/// unsupported high-risk feature, or `exit`/`die`.
#[derive(Clone)]
pub enum Thrown {
    /// An engine-raised error named by class + message (instantiated into a real
    /// Throwable object lazily when caught). Catchable.
    Error {
        class: &'static str,
        message: String,
    },
    /// A user-thrown `Throwable` object (`throw $e`). Catchable.
    Object(crate::ObjectHandle),
    /// A deliberately-unsupported feature (eval/extract/…). Rendered with the
    /// `RHP_UNSUPPORTED` marker, distinct from PHP's "undefined function",
    /// and never swallowable by a user `try/catch`.
    Unsupported {
        feature: String,
        reason: String,
        /// Where the refused construct is (`file`, 1-based `line`), when known.
        location: Option<(String, u32)>,
    },
    /// `exit` / `die`: not an error — unwinds to the SAPI, which exits with this
    /// code. Any already-written output is kept.
    Exit(i32),
}

impl std::fmt::Debug for Thrown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Thrown::Error { class, message } => {
                write!(f, "Error {{ class: {class:?}, message: {message:?} }}")
            }
            Thrown::Object(o) => write!(f, "Object(class #{})", o.borrow().class),
            Thrown::Unsupported {
                feature,
                reason,
                location,
            } => {
                write!(f, "Unsupported {{ feature: {feature:?}, reason: {reason:?}")?;
                if let Some((file, line)) = location {
                    write!(f, ", at: {file:?}:{line}")?;
                }
                write!(f, " }}")
            }
            Thrown::Exit(c) => write!(f, "Exit({c})"),
        }
    }
}

impl Thrown {
    pub fn error(class: &'static str, message: impl Into<String>) -> Thrown {
        Thrown::Error {
            class,
            message: message.into(),
        }
    }
    pub fn type_error(message: impl Into<String>) -> Thrown {
        Thrown::error("TypeError", message)
    }
    pub fn unsupported(feature: impl Into<String>, reason: impl Into<String>) -> Thrown {
        Thrown::Unsupported {
            feature: feature.into(),
            reason: reason.into(),
            location: None,
        }
    }

    /// Stamp a refusal with its source location, unless it already has one
    /// (the innermost construct, which sets it first, wins). Other variants
    /// are returned unchanged.
    pub fn with_location(self, file: impl FnOnce() -> String, line: u32) -> Thrown {
        match self {
            Thrown::Unsupported {
                feature,
                reason,
                location: None,
            } => Thrown::Unsupported {
                feature,
                reason,
                location: Some((file(), line)),
            },
            t => t,
        }
    }

    /// The SAPI's one-line report for a refusal (`None` for other variants):
    /// `RHP_UNSUPPORTED: <feature> is not supported (<reason>) in <file> on
    /// line <n>`. Tooling greps the `RHP_UNSUPPORTED: ` prefix and clusters on
    /// the text before ` (`, so the location goes last.
    pub fn unsupported_message(&self) -> Option<String> {
        let Thrown::Unsupported {
            feature,
            reason,
            location,
        } = self
        else {
            return None;
        };
        let mut msg = format!("RHP_UNSUPPORTED: {feature} is not supported ({reason})");
        if let Some((file, line)) = location {
            if !file.is_empty() && *line > 0 {
                msg.push_str(&format!(" in {file} on line {line}"));
            } else if !file.is_empty() {
                msg.push_str(&format!(" in {file}"));
            }
        }
        Some(msg)
    }

    /// Catchable by a PHP `try/catch`? Real errors and thrown objects are;
    /// refusals and `exit` unwind straight to the SAPI.
    pub fn is_catchable(&self) -> bool {
        matches!(self, Thrown::Error { .. } | Thrown::Object(_))
    }
}

#[cfg(test)]
mod tests;
