//! ext/standard/metaphone.c: `metaphone()`, Michael G Schwern's metaphone
//! as php ships it (traditional mode). The word is read as a C string: a
//! NUL byte ends it.

use rhp_values::{conv, Value};

use crate::{Host, Thrown};

/// metachar.h: 1 vowel, 2 unchanged, 4 affects H, 8 makes C/G soft,
/// 16 stops GH from becoming F.
const CODES: [u8; 26] = [
    1, 16, 4, 16, 9, 2, 4, 16, 9, 2, 0, 2, 2, 2, 1, 4, 0, 2, 4, 4, 1, 0, 0, 0, 8, 0,
];

fn encode(c: u8) -> u8 {
    if c.is_ascii_uppercase() {
        CODES[(c - b'A') as usize]
    } else {
        0
    }
}
fn is_vowel(c: u8) -> bool {
    encode(c) & 1 != 0
}
fn affect_h(c: u8) -> bool {
    encode(c) & 4 != 0
}
fn make_soft(c: u8) -> bool {
    encode(c) & 8 != 0
}
fn no_gh_to_f(c: u8) -> bool {
    encode(c) & 16 != 0
}

const SH: u8 = b'X';
const TH: u8 = b'0';

fn metaphone(word: &[u8], max_phonemes: usize) -> Vec<u8> {
    let raw = |i: usize| word.get(i).copied().unwrap_or(0);
    let up = |i: usize| raw(i).to_ascii_uppercase();
    let mut out = Vec::new();
    let mut w = 0usize;
    // Find the first letter.
    while !raw(w).is_ascii_alphabetic() {
        if raw(w) == 0 {
            return out;
        }
        w += 1;
    }
    let next = |w: usize| up(w + 1);
    let after_next = |w: usize| if raw(w + 1) != 0 { up(w + 2) } else { 0 };
    let look_back = |w: usize, n: usize| if w >= n { up(w - n) } else { 0 };
    // Lookahead: stops at the end of the string.
    let look_ahead = |w: usize, n: usize| {
        let mut i = 0;
        while raw(w + i) != 0 && i < n {
            i += 1;
        }
        up(w + i)
    };
    let c = up(w);
    match c {
        b'A' => {
            if next(w) == b'E' {
                out.push(b'E');
                w += 2;
            } else {
                out.push(b'A');
                w += 1;
            }
        }
        b'G' | b'K' | b'P' => {
            if next(w) == b'N' {
                out.push(b'N');
                w += 2;
            }
        }
        b'W' => {
            let n = next(w);
            if n == b'R' {
                out.push(b'R');
                w += 2;
            } else if n == b'H' || is_vowel(n) {
                out.push(b'W');
                w += 2;
            }
        }
        b'X' => {
            out.push(b'S');
            w += 1;
        }
        b'E' | b'I' | b'O' | b'U' => {
            out.push(c);
            w += 1;
        }
        _ => {}
    }
    while raw(w) != 0 && (max_phonemes == 0 || out.len() < max_phonemes) {
        let mut skip = 0;
        if !raw(w).is_ascii_alphabetic() {
            w += 1;
            continue;
        }
        let c = up(w);
        let prev = look_back(w, 1);
        if c == prev && c != b'C' {
            w += 1;
            continue;
        }
        match c {
            b'B' => {
                if prev != b'M' {
                    out.push(b'B');
                }
            }
            b'C' => {
                let n = next(w);
                if make_soft(n) {
                    if n == b'I' && after_next(w) == b'A' {
                        out.push(SH);
                    } else if prev != b'S' {
                        out.push(b'S');
                    }
                } else if n == b'H' {
                    out.push(SH);
                    skip += 1;
                } else {
                    out.push(b'K');
                }
            }
            b'D' => {
                if next(w) == b'G' && make_soft(after_next(w)) {
                    out.push(b'J');
                    skip += 1;
                } else {
                    out.push(b'T');
                }
            }
            b'G' => {
                let n = next(w);
                if n == b'H' {
                    if !(no_gh_to_f(look_back(w, 3)) || look_back(w, 4) == b'H') {
                        out.push(b'F');
                        skip += 1;
                    }
                } else if n == b'N' {
                    let an = after_next(w);
                    if !an.is_ascii_alphabetic() || (an == b'E' && look_ahead(w, 3) == b'D') {
                        // dropped
                    } else {
                        out.push(b'K');
                    }
                } else if make_soft(n) && prev != b'G' {
                    out.push(b'J');
                } else {
                    out.push(b'K');
                }
            }
            b'H' => {
                if is_vowel(next(w)) && !affect_h(prev) {
                    out.push(b'H');
                }
            }
            b'K' => {
                if prev != b'C' {
                    out.push(b'K');
                }
            }
            b'P' => out.push(if next(w) == b'H' { b'F' } else { b'P' }),
            b'Q' => out.push(b'K'),
            b'S' => {
                let n = next(w);
                if n == b'I' && matches!(after_next(w), b'O' | b'A') {
                    out.push(SH);
                } else if n == b'H' {
                    out.push(SH);
                    skip += 1;
                } else {
                    out.push(b'S');
                }
            }
            b'T' => {
                let n = next(w);
                if n == b'I' && matches!(after_next(w), b'O' | b'A') {
                    out.push(SH);
                } else if n == b'H' {
                    out.push(TH);
                    skip += 1;
                } else if !(n == b'C' && after_next(w) == b'H') {
                    out.push(b'T');
                }
            }
            b'V' => out.push(b'F'),
            b'W' => {
                if is_vowel(next(w)) {
                    out.push(b'W');
                }
            }
            b'X' => out.extend_from_slice(b"KS"),
            b'Y' => {
                if is_vowel(next(w)) {
                    out.push(b'Y');
                }
            }
            b'Z' => out.push(b'S'),
            b'F' | b'J' | b'L' | b'M' | b'N' | b'R' => out.push(c),
            _ => {}
        }
        w += skip + 1;
    }
    out
}

/// `metaphone(string $string, int $max_phonemes = 0): string`.
fn fn_metaphone(_: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let s = conv::to_string_bytes(&args[0]);
    let phones = args.get(1).map(conv::to_int).unwrap_or(0);
    if phones < 0 {
        return Err(Thrown::error(
            "ValueError",
            "metaphone(): Argument #2 ($max_phonemes) must be greater than or equal to 0",
        ));
    }
    Ok(Value::str(metaphone(&s, phones as usize)))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("metaphone", fn_metaphone)]);
}
