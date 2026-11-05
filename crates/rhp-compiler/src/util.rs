//! Small helpers shared by the compiler's modules.

use rhp_values::Thrown;

use crate::Builtins;

/// `dirname()` of a path, matching PHP for the file paths `__FILE__` yields:
/// no slash → ".", a single leading slash → "/", else everything before the
/// last slash.
pub(crate) fn dirname(builtins: &Builtins, path: &[u8]) -> Vec<u8> {
    match path.iter().rposition(|&b| b == b'/') {
        // php (zend_compile `__DIR__`): a "." directory — "Command line
        // code", "Standard input code" — becomes the working directory.
        None => (builtins.cwd)().unwrap_or_else(|| b".".to_vec()),
        Some(0) => b"/".to_vec(),
        Some(i) => path[..i].to_vec(),
    }
}

/// A php compile-time fatal error (E_COMPILE_ERROR) at `line`.
pub(crate) fn compile_error(msg: impl Into<String>, _line: u32) -> Thrown {
    Thrown::error("Error", msg)
}

pub(crate) fn strip_dollar(name: &[u8]) -> &[u8] {
    name.strip_prefix(b"$").unwrap_or(name)
}

/// PHP's superglobals, available in every scope without `global`.
pub(crate) fn is_superglobal(name: &[u8]) -> bool {
    matches!(
        name,
        b"GLOBALS"
            | b"_SERVER"
            | b"_GET"
            | b"_POST"
            | b"_REQUEST"
            | b"_SESSION"
            | b"_ENV"
            | b"_COOKIE"
            | b"_FILES"
    )
}
