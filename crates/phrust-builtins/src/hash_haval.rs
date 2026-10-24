//! HAVAL (PHP's `haval<bits>,<passes>` — bits ∈ {128,160,192,224,256},
//! passes ∈ {3,4,5}).
//!
//! No maintained Rust crate matches PHP 1:1, so this is a hand-rolled, faithful
//! port of PHP's `ext/hash/hash_haval.c`. HAVAL keeps a 256-bit (8 × u32) state,
//! processes 1024-bit (128-byte) blocks with 3/4/5 passes of 32 steps each, and
//! — for the sub-256-bit digests — folds ("tailors") the final 256-bit state
//! down to the requested width. All words are little-endian, and the padding
//! begins with a `0x01` byte (not `0x80`), followed by a version/passes/length
//! trailer; these quirks are reproduced exactly so the output is byte-identical
//! to `php`'s `hash()`.

// ---------------------------------------------------------------------------
// constants (digits of the fractional part of pi; identical to PHP's tables)
// ---------------------------------------------------------------------------

/// Initial 256-bit state (first 8 pi words).
const D0: [u32; 8] = [
    0x243F6A88, 0x85A308D3, 0x13198A2E, 0x03707344, 0xA4093822, 0x299F31D0, 0x082EFA98, 0xEC4E6C89,
];

const K2: [u32; 32] = [
    0x452821E6, 0x38D01377, 0xBE5466CF, 0x34E90C6C, 0xC0AC29B7, 0xC97C50DD, 0x3F84D5B5, 0xB5470917,
    0x9216D5D9, 0x8979FB1B, 0xD1310BA6, 0x98DFB5AC, 0x2FFD72DB, 0xD01ADFB7, 0xB8E1AFED, 0x6A267E96,
    0xBA7C9045, 0xF12C7F99, 0x24A19947, 0xB3916CF7, 0x0801F2E2, 0x858EFC16, 0x636920D8, 0x71574E69,
    0xA458FEA3, 0xF4933D7E, 0x0D95748F, 0x728EB658, 0x718BCD58, 0x82154AEE, 0x7B54A41D, 0xC25A59B5,
];

const K3: [u32; 32] = [
    0x9C30D539, 0x2AF26013, 0xC5D1B023, 0x286085F0, 0xCA417918, 0xB8DB38EF, 0x8E79DCB0, 0x603A180E,
    0x6C9E0E8B, 0xB01E8A3E, 0xD71577C1, 0xBD314B27, 0x78AF2FDA, 0x55605C60, 0xE65525F3, 0xAA55AB94,
    0x57489862, 0x63E81440, 0x55CA396A, 0x2AAB10B6, 0xB4CC5C34, 0x1141E8CE, 0xA15486AF, 0x7C72E993,
    0xB3EE1411, 0x636FBC2A, 0x2BA9C55D, 0x741831F6, 0xCE5C3E16, 0x9B87931E, 0xAFD6BA33, 0x6C24CF5C,
];

const K4: [u32; 32] = [
    0x7A325381, 0x28958677, 0x3B8F4898, 0x6B4BB9AF, 0xC4BFE81B, 0x66282193, 0x61D809CC, 0xFB21A991,
    0x487CAC60, 0x5DEC8032, 0xEF845D5D, 0xE98575B1, 0xDC262302, 0xEB651B88, 0x23893E81, 0xD396ACC5,
    0x0F6D6FF3, 0x83F44239, 0x2E0B4482, 0xA4842004, 0x69C8F04A, 0x9E1F9B5E, 0x21C66842, 0xF6E96C9A,
    0x670C9C61, 0xABD388F0, 0x6A51A0D2, 0xD8542F68, 0x960FA728, 0xAB5133A3, 0x6EEF0B6C, 0x137A3BE4,
];

const K5: [u32; 32] = [
    0xBA3BF050, 0x7EFB2A98, 0xA1F1651D, 0x39AF0176, 0x66CA593E, 0x82430E88, 0x8CEE8619, 0x456F9FB4,
    0x7D84A5C3, 0x3B8B5EBE, 0xE06F75D8, 0x85C12073, 0x401A449F, 0x56C16AA6, 0x4ED3AA62, 0x363F7706,
    0x1BFEDF72, 0x429B023D, 0x37D0D724, 0xD00A1248, 0xDB0FEAD3, 0x49F1C09B, 0x075372C9, 0x80991B7B,
    0x25D479D8, 0xF6E8DEF7, 0xE3FE501A, 0xB6794C3B, 0x976CE0BD, 0x04C006BA, 0xC1A94FB6, 0x409F60C4,
];

/// Per-pass message-word orderings for passes 2..5 (pass 1 reads words in order).
const I2: [usize; 32] = [
    5, 14, 26, 18, 11, 28, 7, 16, 0, 23, 20, 22, 1, 10, 4, 8, 30, 3, 21, 9, 17, 24, 29, 6, 19, 12,
    15, 13, 2, 25, 31, 27,
];
const I3: [usize; 32] = [
    19, 9, 4, 20, 28, 17, 8, 22, 29, 14, 25, 12, 24, 30, 16, 26, 31, 15, 7, 3, 1, 0, 18, 27, 13, 6,
    21, 10, 23, 11, 5, 2,
];
const I4: [usize; 32] = [
    24, 4, 0, 14, 2, 7, 28, 23, 26, 6, 30, 20, 18, 25, 19, 3, 22, 11, 31, 21, 8, 27, 12, 9, 1, 29,
    5, 15, 17, 10, 16, 13,
];
const I5: [usize; 32] = [
    27, 3, 21, 26, 17, 11, 20, 29, 19, 0, 12, 7, 13, 8, 31, 10, 5, 9, 14, 30, 18, 6, 28, 24, 2, 23,
    16, 22, 4, 1, 25, 15,
];

/// `M[k][i] = (k - i) mod 8`: the state-word selectors used by each step.
const M: [[usize; 32]; 8] = [
    [0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1],
    [1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2],
    [2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3],
    [3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4],
    [4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5],
    [5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6],
    [6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7],
    [7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0, 7, 6, 5, 4, 3, 2, 1, 0],
];

// ---------------------------------------------------------------------------
// boolean round functions (arguments named x6..x0, matching the spec)
// ---------------------------------------------------------------------------

#[inline]
fn f1(x6: u32, x5: u32, x4: u32, x3: u32, x2: u32, x1: u32, x0: u32) -> u32 {
    (x1 & x4) ^ (x2 & x5) ^ (x3 & x6) ^ (x0 & x1) ^ x0
}

#[inline]
fn f2(x6: u32, x5: u32, x4: u32, x3: u32, x2: u32, x1: u32, x0: u32) -> u32 {
    (x1 & x2 & x3)
        ^ (x2 & x4 & x5)
        ^ (x1 & x2)
        ^ (x1 & x4)
        ^ (x2 & x6)
        ^ (x3 & x5)
        ^ (x4 & x5)
        ^ (x0 & x2)
        ^ x0
}

#[inline]
fn f3(x6: u32, x5: u32, x4: u32, x3: u32, x2: u32, x1: u32, x0: u32) -> u32 {
    (x1 & x2 & x3) ^ (x1 & x4) ^ (x2 & x5) ^ (x3 & x6) ^ (x0 & x3) ^ x0
}

#[inline]
fn f4(x6: u32, x5: u32, x4: u32, x3: u32, x2: u32, x1: u32, x0: u32) -> u32 {
    (x1 & x2 & x3)
        ^ (x2 & x4 & x5)
        ^ (x3 & x4 & x6)
        ^ (x1 & x4)
        ^ (x2 & x6)
        ^ (x3 & x4)
        ^ (x3 & x5)
        ^ (x3 & x6)
        ^ (x4 & x5)
        ^ (x4 & x6)
        ^ (x0 & x4)
        ^ x0
}

#[inline]
fn f5(x6: u32, x5: u32, x4: u32, x3: u32, x2: u32, x1: u32, x0: u32) -> u32 {
    (x1 & x4) ^ (x2 & x5) ^ (x3 & x6) ^ (x0 & x1 & x2 & x3) ^ (x0 & x5) ^ x0
}

/// `rotate_right` on a 32-bit word.
#[inline]
fn rotr(x: u32, n: u32) -> u32 {
    x.rotate_right(n)
}

// ---------------------------------------------------------------------------
// block transform (3, 4 or 5 passes)
// ---------------------------------------------------------------------------

/// Apply one 128-byte block to the 8-word `state`, using `passes` ∈ {3,4,5}.
fn transform(state: &mut [u32; 8], block: &[u8; 128], passes: usize) {
    // Decode 32 little-endian u32 message words.
    let mut x = [0u32; 32];
    for (i, w) in x.iter_mut().enumerate() {
        let b = i * 4;
        *w = (block[b] as u32)
            | ((block[b + 1] as u32) << 8)
            | ((block[b + 2] as u32) << 16)
            | ((block[b + 3] as u32) << 24);
    }

    let mut e = *state;

    // The updated word at step i is e[7 - (i % 8)]; `w` is ROTR(e[M7[i]], 11).
    macro_rules! step {
        ($i:expr, $fval:expr, $word:expr) => {{
            let i = $i;
            let u = 7 - (i % 8);
            e[u] = rotr($fval, 7)
                .wrapping_add(rotr(e[M[7][i]], 11))
                .wrapping_add($word);
        }};
    }

    // Pass 1 (no round constant; words in natural order).
    for i in 0..32 {
        let v = match passes {
            3 => f1(e[M[1][i]], e[M[0][i]], e[M[3][i]], e[M[5][i]], e[M[6][i]], e[M[2][i]], e[M[4][i]]),
            4 => f1(e[M[2][i]], e[M[6][i]], e[M[1][i]], e[M[4][i]], e[M[5][i]], e[M[3][i]], e[M[0][i]]),
            _ => f1(e[M[3][i]], e[M[4][i]], e[M[1][i]], e[M[0][i]], e[M[5][i]], e[M[2][i]], e[M[6][i]]),
        };
        step!(i, v, x[i]);
    }

    // Pass 2.
    for i in 0..32 {
        let v = match passes {
            3 => f2(e[M[4][i]], e[M[2][i]], e[M[1][i]], e[M[0][i]], e[M[5][i]], e[M[3][i]], e[M[6][i]]),
            4 => f2(e[M[3][i]], e[M[5][i]], e[M[2][i]], e[M[0][i]], e[M[1][i]], e[M[6][i]], e[M[4][i]]),
            _ => f2(e[M[6][i]], e[M[2][i]], e[M[1][i]], e[M[0][i]], e[M[3][i]], e[M[4][i]], e[M[5][i]]),
        };
        step!(i, v, x[I2[i]].wrapping_add(K2[i]));
    }

    // Pass 3.
    for i in 0..32 {
        let v = match passes {
            3 => f3(e[M[6][i]], e[M[1][i]], e[M[2][i]], e[M[3][i]], e[M[4][i]], e[M[5][i]], e[M[0][i]]),
            4 => f3(e[M[1][i]], e[M[4][i]], e[M[3][i]], e[M[6][i]], e[M[0][i]], e[M[2][i]], e[M[5][i]]),
            _ => f3(e[M[2][i]], e[M[6][i]], e[M[0][i]], e[M[4][i]], e[M[3][i]], e[M[1][i]], e[M[5][i]]),
        };
        step!(i, v, x[I3[i]].wrapping_add(K3[i]));
    }

    // Pass 4 (4- and 5-pass variants only).
    if passes >= 4 {
        for i in 0..32 {
            let v = match passes {
                4 => f4(e[M[6][i]], e[M[4][i]], e[M[0][i]], e[M[5][i]], e[M[2][i]], e[M[1][i]], e[M[3][i]]),
                _ => f4(e[M[1][i]], e[M[5][i]], e[M[3][i]], e[M[2][i]], e[M[0][i]], e[M[4][i]], e[M[6][i]]),
            };
            step!(i, v, x[I4[i]].wrapping_add(K4[i]));
        }
    }

    // Pass 5 (5-pass variant only).
    if passes >= 5 {
        for i in 0..32 {
            let v = f5(
                e[M[2][i]], e[M[5][i]], e[M[0][i]], e[M[6][i]], e[M[4][i]], e[M[3][i]], e[M[1][i]],
            );
            step!(i, v, x[I5[i]].wrapping_add(K5[i]));
        }
    }

    for i in 0..8 {
        state[i] = state[i].wrapping_add(e[i]);
    }
}

// ---------------------------------------------------------------------------
// output tailoring: fold the 256-bit state down to the requested width
// ---------------------------------------------------------------------------

fn tailor_128(s: &mut [u32; 8]) {
    s[3] = s[3]
        .wrapping_add((s[7] & 0xFF000000) | (s[6] & 0x00FF0000) | (s[5] & 0x0000FF00) | (s[4] & 0x000000FF));
    s[2] = s[2].wrapping_add(
        (((s[7] & 0x00FF0000) | (s[6] & 0x0000FF00) | (s[5] & 0x000000FF)) << 8)
            | ((s[4] & 0xFF000000) >> 24),
    );
    s[1] = s[1].wrapping_add(
        (((s[7] & 0x0000FF00) | (s[6] & 0x000000FF)) << 16)
            | (((s[5] & 0xFF000000) | (s[4] & 0x00FF0000)) >> 16),
    );
    s[0] = s[0].wrapping_add(
        ((s[7] & 0x000000FF) << 24)
            | (((s[6] & 0xFF000000) | (s[5] & 0x00FF0000) | (s[4] & 0x0000FF00)) >> 8),
    );
}

fn tailor_160(s: &mut [u32; 8]) {
    s[4] = s[4]
        .wrapping_add(((s[7] & 0xFE000000) | (s[6] & 0x01F80000) | (s[5] & 0x0007F000)) >> 12);
    s[3] = s[3]
        .wrapping_add(((s[7] & 0x01F80000) | (s[6] & 0x0007F000) | (s[5] & 0x00000FC0)) >> 6);
    s[2] = s[2].wrapping_add((s[7] & 0x0007F000) | (s[6] & 0x00000FC0) | (s[5] & 0x0000003F));
    s[1] = s[1].wrapping_add(rotr(
        (s[7] & 0x00000FC0) | (s[6] & 0x0000003F) | (s[5] & 0xFE000000),
        25,
    ));
    s[0] = s[0].wrapping_add(rotr(
        (s[7] & 0x0000003F) | (s[6] & 0xFE000000) | (s[5] & 0x01F80000),
        19,
    ));
}

fn tailor_192(s: &mut [u32; 8]) {
    s[5] = s[5].wrapping_add(((s[7] & 0xFC000000) | (s[6] & 0x03E00000)) >> 21);
    s[4] = s[4].wrapping_add(((s[7] & 0x03E00000) | (s[6] & 0x001F0000)) >> 16);
    s[3] = s[3].wrapping_add(((s[7] & 0x001F0000) | (s[6] & 0x0000FC00)) >> 10);
    s[2] = s[2].wrapping_add(((s[7] & 0x0000FC00) | (s[6] & 0x000003E0)) >> 5);
    s[1] = s[1].wrapping_add((s[7] & 0x000003E0) | (s[6] & 0x0000001F));
    s[0] = s[0].wrapping_add(rotr((s[7] & 0x0000001F) | (s[6] & 0xFC000000), 26));
}

fn tailor_224(s: &mut [u32; 8]) {
    s[6] = s[6].wrapping_add(s[7] & 0x0000000F);
    s[5] = s[5].wrapping_add((s[7] >> 4) & 0x0000001F);
    s[4] = s[4].wrapping_add((s[7] >> 9) & 0x0000000F);
    s[3] = s[3].wrapping_add((s[7] >> 13) & 0x0000001F);
    s[2] = s[2].wrapping_add((s[7] >> 18) & 0x0000000F);
    s[1] = s[1].wrapping_add((s[7] >> 22) & 0x0000001F);
    s[0] = s[0].wrapping_add((s[7] >> 27) & 0x0000001F);
}

// ---------------------------------------------------------------------------
// public entry point
// ---------------------------------------------------------------------------

/// Compute the HAVAL digest of `data`.
///
/// `bits` is the output size in bits (128/160/192/224/256) and `passes` is the
/// number of rounds (3/4/5). Returns `bits / 8` raw digest bytes in PHP's byte
/// order.
pub fn haval(data: &[u8], bits: usize, passes: usize) -> Vec<u8> {
    let mut state = D0;

    // Full blocks of the real message.
    let mut i = 0;
    while i + 128 <= data.len() {
        let block: &[u8; 128] = data[i..i + 128].try_into().unwrap();
        transform(&mut state, block, passes);
        i += 128;
    }

    // Build the final (tail) block(s): remaining bytes + padding + trailer.
    let bitlen: u64 = (data.len() as u64).wrapping_shl(3);
    let index = data.len() & 0x7f; // bytes already in the partial block
    let pad_len = if index < 118 { 118 - index } else { 246 - index };

    let mut tail: Vec<u8> = data[i..].to_vec();
    // Padding: a single 0x01 byte followed by zeros.
    tail.push(0x01);
    tail.extend(std::iter::repeat(0u8).take(pad_len - 1));
    // Trailer: version|passes|output byte, high output bits, then 64-bit LE length.
    let output = bits as u32;
    tail.push(
        ((0x01u32 & 0x07) | ((passes as u32 & 0x07) << 3) | ((output & 0x03) << 6)) as u8,
    );
    tail.push((output >> 2) as u8);
    tail.extend_from_slice(&bitlen.to_le_bytes());

    debug_assert_eq!(tail.len() % 128, 0);
    for chunk in tail.chunks_exact(128) {
        let block: &[u8; 128] = chunk.try_into().unwrap();
        transform(&mut state, block, passes);
    }

    // Fold the state down to the requested width.
    match bits {
        128 => tailor_128(&mut state),
        160 => tailor_160(&mut state),
        192 => tailor_192(&mut state),
        224 => tailor_224(&mut state),
        _ => {} // 256: no tailoring
    }

    // Emit `bits / 8` bytes as little-endian words.
    let words = bits / 32;
    let mut out = Vec::with_capacity(bits / 8);
    for &w in &state[..words] {
        out.extend_from_slice(&w.to_le_bytes());
    }
    out
}
