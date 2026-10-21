//! The interpreter's unwinding type, at the leaf of the crate graph so that
//! `phrust-semantics` (operators), `phrust-builtins`, and both execution tiers
//! can all return `Result<_, Thrown>` without a dependency cycle.

/// Unwinds the interpreter: a real PHP `Throwable`, an engine refusal for an
/// unsupported high-risk feature, or `exit`/`die`.
#[derive(Debug, Clone)]
pub enum Thrown {
    /// A real PHP error/exception (caught by `try/catch` from phase 5 on).
    Error { class: &'static str, message: String },
    /// A deliberately-unsupported feature (eval/extract/…). Rendered with the
    /// `PHRUST_UNSUPPORTED` marker, distinct from PHP's "undefined function",
    /// and never swallowable by a user `try/catch`.
    Unsupported { feature: String, reason: &'static str },
    /// `exit` / `die`: not an error — unwinds to the SAPI, which exits with this
    /// code. Any already-written output is kept.
    Exit(i32),
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

    /// Catchable by a PHP `try/catch`? Only real `Error`s are; refusals and
    /// `exit` unwind straight to the SAPI.
    pub fn is_catchable(&self) -> bool {
        matches!(self, Thrown::Error { .. })
    }
}
