//! PHP's 4-pass Tiger variant (`tiger128,4` / `tiger160,4` / `tiger192,4`).
//!
//! The standard 3-pass Tiger is provided by the `tiger` crate; the 4-pass form
//! is a PHP/mhash extension with no crate, so it is hand-rolled here over the
//! shared Tiger S-boxes.
//!
//! NOTE: stub — filled in and verified against `php` via `phrust-difftest`.

pub fn tiger4(_data: &[u8], width: usize) -> Vec<u8> {
    vec![0u8; width]
}
