//! HAVAL (PHP's `haval<bits>,<passes>` — bits ∈ {128,160,192,224,256},
//! passes ∈ {3,4,5}).
//!
//! No maintained Rust crate exists, so this is hand-rolled from the HAVAL spec.
//!
//! NOTE: stub — filled in and verified against `php` via `phrust-difftest`.

/// `bits` is the output size in bits; `passes` the number of rounds (3/4/5).
pub fn haval(_data: &[u8], bits: usize, _passes: usize) -> Vec<u8> {
    vec![0u8; bits / 8]
}
