//! Snefru-2.0 256-bit hash (PHP's `snefru` and `snefru256`, both 256-bit).
//!
//! No maintained Rust crate exists, so this is hand-rolled over the standard
//! Snefru S-box table.
//!
//! NOTE: stub — filled in and verified against `php` via `phrust-difftest`.

pub fn snefru256(_data: &[u8]) -> [u8; 32] {
    [0u8; 32]
}
