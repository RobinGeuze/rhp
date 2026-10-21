//! The interpreter's unwinding type, at the leaf of the crate graph so that
//! `phrust-semantics` (operators), `phrust-builtins`, and both execution tiers
//! can all return `Result<_, Thrown>` without a dependency cycle.

/// Unwinds the interpreter: a real PHP `Throwable`, an engine refusal for an
/// unsupported high-risk feature, or `exit`/`die`.
#[derive(Clone)]
pub enum Thrown {
    /// An engine-raised error named by class + message (instantiated into a real
    /// Throwable object lazily when caught). Catchable.
    Error { class: &'static str, message: String },
    /// A user-thrown `Throwable` object (`throw $e`). Catchable.
    Object(crate::ObjectHandle),
    /// A deliberately-unsupported feature (eval/extract/…). Rendered with the
    /// `PHRUST_UNSUPPORTED` marker, distinct from PHP's "undefined function",
    /// and never swallowable by a user `try/catch`.
    Unsupported { feature: String, reason: &'static str },
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
            Thrown::Unsupported { feature, reason } => {
                write!(f, "Unsupported {{ feature: {feature:?}, reason: {reason:?} }}")
            }
            Thrown::Exit(c) => write!(f, "Exit({c})"),
        }
    }
}

impl Thrown {
    pub fn error(class: &'static str, message: impl Into<String>) -> Thrown {
        Thrown::Error { class, message: message.into() }
    }
    pub fn type_error(message: impl Into<String>) -> Thrown {
        Thrown::error("TypeError", message)
    }
    pub fn unsupported(feature: impl Into<String>, reason: &'static str) -> Thrown {
        Thrown::Unsupported { feature: feature.into(), reason }
    }

    /// Catchable by a PHP `try/catch`? Real errors and thrown objects are;
    /// refusals and `exit` unwind straight to the SAPI.
    pub fn is_catchable(&self) -> bool {
        matches!(self, Thrown::Error { .. } | Thrown::Object(_))
    }
}
