//! lexbor, as ext/lexbor bundles it: the archive and its allocator setup.
//! ext/dom's glue (rhp-builtins) compiles against the same headers and
//! symbol prefix and calls lexbor directly.

use std::sync::Once;

extern "C" {
    fn rhp_lexbor_setup();
}

/// ext/lexbor's MINIT: lexbor allocates with emalloc. Idempotent.
pub fn setup() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe { rhp_lexbor_setup() });
}
