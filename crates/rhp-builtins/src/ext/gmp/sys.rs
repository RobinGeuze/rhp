//! The part of libgmp's API ext/gmp calls, declared by hand (no bindgen),
//! and [`Mpz`], an owned `mpz_t`.
//!
//! gmp.h names every function through a macro (`mpz_add` is `__gmpz_add`);
//! the extern names here are the symbols. `mpz_sgn` is a macro over the
//! struct (and [`Mpz::limbs`] reads it as `mpz_size` does), so
//! [`MpzStruct`] mirrors `__mpz_struct`; `shim.c` checks that layout
//! against the gmp.h build.rs linked. `mpz_cmp_si`'s macro is bypassed by
//! calling `__gmpz_cmp_si`. libgmp is LGPL and linked dynamically
//! (docs/design/libraries.md).
//!
//! libgmp aborts the process when a number outgrows an `mpz_t` (more than
//! `INT_MAX` limbs) or memory runs out: callers check sizes first.

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_int, c_long, c_ulong, c_void, CStr};

pub type mp_limb_t = u64;
pub type mp_bitcnt_t = c_ulong;

/// `__mpz_struct`.
#[repr(C)]
pub struct MpzStruct {
    alloc: c_int,
    size: c_int,
    d: *mut mp_limb_t,
}

/// `__gmp_randstate_struct`, opaque: an `mpz_t`, an enum and a pointer.
#[repr(C, align(8))]
pub struct RandStruct([u8; 32]);

type Z = *mut MpzStruct;
type Zc = *const MpzStruct;
type R = *mut RandStruct;

/// `void (*)(mpz_ptr, mpz_srcptr, mpz_srcptr)`.
pub type BinOp = unsafe extern "C" fn(Z, Zc, Zc);
/// `void (*)(mpz_ptr, mpz_srcptr)`.
pub type UnOp = unsafe extern "C" fn(Z, Zc);
/// `void (*)(mpz_ptr, mpz_ptr, mpz_srcptr, mpz_srcptr)`.
pub type DivQr = unsafe extern "C" fn(Z, Z, Zc, Zc);
/// `void (*)(mpz_ptr, mpz_srcptr, mp_bitcnt_t)`.
pub type UiOp = unsafe extern "C" fn(Z, Zc, c_ulong);

// build/native.rs links libgmp.
extern "C" {
    #[link_name = "__gmp_version"]
    static gmp_version: *const c_char;

    #[link_name = "__gmpz_init"]
    fn mpz_init(z: Z);
    #[link_name = "__gmpz_clear"]
    fn mpz_clear(z: Z);
    #[link_name = "__gmpz_set"]
    fn mpz_set(r: Z, a: Zc);
    #[link_name = "__gmpz_set_si"]
    fn mpz_set_si(r: Z, v: c_long);
    #[link_name = "__gmpz_set_ui"]
    fn mpz_set_ui(r: Z, v: c_ulong);
    #[link_name = "__gmpz_set_str"]
    fn mpz_set_str(r: Z, s: *const c_char, base: c_int) -> c_int;
    #[link_name = "__gmpz_get_str"]
    fn mpz_get_str(s: *mut c_char, base: c_int, a: Zc) -> *mut c_char;
    #[link_name = "__gmpz_get_si"]
    fn mpz_get_si(a: Zc) -> c_long;
    #[link_name = "__gmpz_get_ui"]
    fn mpz_get_ui(a: Zc) -> c_ulong;
    #[link_name = "__gmpz_get_d"]
    fn mpz_get_d(a: Zc) -> f64;
    #[link_name = "__gmpz_fits_slong_p"]
    fn mpz_fits_slong_p(a: Zc) -> c_int;
    #[link_name = "__gmpz_sizeinbase"]
    fn mpz_sizeinbase(a: Zc, base: c_int) -> usize;
    #[link_name = "__gmpz_cmp"]
    fn mpz_cmp(a: Zc, b: Zc) -> c_int;
    #[link_name = "__gmpz_cmp_si"]
    fn mpz_cmp_si(a: Zc, b: c_long) -> c_int;

    #[link_name = "__gmpz_add"]
    pub fn mpz_add(r: Z, a: Zc, b: Zc);
    #[link_name = "__gmpz_add_ui"]
    fn mpz_add_ui(r: Z, a: Zc, b: c_ulong);
    #[link_name = "__gmpz_sub"]
    pub fn mpz_sub(r: Z, a: Zc, b: Zc);
    #[link_name = "__gmpz_mul"]
    pub fn mpz_mul(r: Z, a: Zc, b: Zc);
    #[link_name = "__gmpz_tdiv_q"]
    pub fn mpz_tdiv_q(r: Z, a: Zc, b: Zc);
    #[link_name = "__gmpz_tdiv_r"]
    pub fn mpz_tdiv_r(r: Z, a: Zc, b: Zc);
    #[link_name = "__gmpz_tdiv_qr"]
    pub fn mpz_tdiv_qr(q: Z, r: Z, a: Zc, b: Zc);
    #[link_name = "__gmpz_cdiv_q"]
    pub fn mpz_cdiv_q(r: Z, a: Zc, b: Zc);
    #[link_name = "__gmpz_cdiv_r"]
    pub fn mpz_cdiv_r(r: Z, a: Zc, b: Zc);
    #[link_name = "__gmpz_cdiv_qr"]
    pub fn mpz_cdiv_qr(q: Z, r: Z, a: Zc, b: Zc);
    #[link_name = "__gmpz_fdiv_q"]
    pub fn mpz_fdiv_q(r: Z, a: Zc, b: Zc);
    #[link_name = "__gmpz_fdiv_r"]
    pub fn mpz_fdiv_r(r: Z, a: Zc, b: Zc);
    #[link_name = "__gmpz_fdiv_qr"]
    pub fn mpz_fdiv_qr(q: Z, r: Z, a: Zc, b: Zc);
    #[link_name = "__gmpz_mod"]
    pub fn mpz_mod(r: Z, a: Zc, b: Zc);
    #[link_name = "__gmpz_divexact"]
    pub fn mpz_divexact(r: Z, a: Zc, b: Zc);
    #[link_name = "__gmpz_gcd"]
    pub fn mpz_gcd(r: Z, a: Zc, b: Zc);
    #[link_name = "__gmpz_lcm"]
    pub fn mpz_lcm(r: Z, a: Zc, b: Zc);
    #[link_name = "__gmpz_and"]
    pub fn mpz_and(r: Z, a: Zc, b: Zc);
    #[link_name = "__gmpz_ior"]
    pub fn mpz_ior(r: Z, a: Zc, b: Zc);
    #[link_name = "__gmpz_xor"]
    pub fn mpz_xor(r: Z, a: Zc, b: Zc);

    #[link_name = "__gmpz_neg"]
    pub fn mpz_neg(r: Z, a: Zc);
    #[link_name = "__gmpz_abs"]
    pub fn mpz_abs(r: Z, a: Zc);
    #[link_name = "__gmpz_com"]
    pub fn mpz_com(r: Z, a: Zc);
    #[link_name = "__gmpz_sqrt"]
    pub fn mpz_sqrt(r: Z, a: Zc);
    #[link_name = "__gmpz_nextprime"]
    pub fn mpz_nextprime(r: Z, a: Zc);
    #[link_name = "__gmpz_sqrtrem"]
    fn mpz_sqrtrem(s: Z, r: Z, a: Zc);

    #[link_name = "__gmpz_fac_ui"]
    fn mpz_fac_ui(r: Z, n: c_ulong);
    #[link_name = "__gmpz_bin_ui"]
    fn mpz_bin_ui(r: Z, n: Zc, k: c_ulong);
    #[link_name = "__gmpz_pow_ui"]
    pub fn mpz_pow_ui(r: Z, a: Zc, e: c_ulong);
    #[link_name = "__gmpz_mul_2exp"]
    pub fn mpz_mul_2exp(r: Z, a: Zc, n: mp_bitcnt_t);
    #[link_name = "__gmpz_fdiv_q_2exp"]
    pub fn mpz_fdiv_q_2exp(r: Z, a: Zc, n: mp_bitcnt_t);
    #[link_name = "__gmpz_powm"]
    fn mpz_powm(r: Z, b: Zc, e: Zc, m: Zc);
    #[link_name = "__gmpz_root"]
    fn mpz_root(r: Z, a: Zc, n: c_ulong) -> c_int;
    #[link_name = "__gmpz_rootrem"]
    fn mpz_rootrem(r: Z, rem: Z, a: Zc, n: c_ulong);

    #[link_name = "__gmpz_perfect_square_p"]
    fn mpz_perfect_square_p(a: Zc) -> c_int;
    #[link_name = "__gmpz_perfect_power_p"]
    fn mpz_perfect_power_p(a: Zc) -> c_int;
    #[link_name = "__gmpz_probab_prime_p"]
    fn mpz_probab_prime_p(a: Zc, reps: c_int) -> c_int;
    #[link_name = "__gmpz_gcdext"]
    fn mpz_gcdext(g: Z, s: Z, t: Z, a: Zc, b: Zc);
    #[link_name = "__gmpz_invert"]
    fn mpz_invert(r: Z, a: Zc, m: Zc) -> c_int;
    // gmp.h: `#define mpz_kronecker mpz_jacobi`, `#define mpz_legendre
    // mpz_jacobi`.
    #[link_name = "__gmpz_jacobi"]
    fn mpz_jacobi(a: Zc, b: Zc) -> c_int;

    #[link_name = "__gmpz_setbit"]
    fn mpz_setbit(r: Z, i: mp_bitcnt_t);
    #[link_name = "__gmpz_clrbit"]
    fn mpz_clrbit(r: Z, i: mp_bitcnt_t);
    #[link_name = "__gmpz_tstbit"]
    fn mpz_tstbit(a: Zc, i: mp_bitcnt_t) -> c_int;
    #[link_name = "__gmpz_popcount"]
    fn mpz_popcount(a: Zc) -> mp_bitcnt_t;
    #[link_name = "__gmpz_hamdist"]
    fn mpz_hamdist(a: Zc, b: Zc) -> mp_bitcnt_t;
    #[link_name = "__gmpz_scan0"]
    fn mpz_scan0(a: Zc, start: mp_bitcnt_t) -> mp_bitcnt_t;
    #[link_name = "__gmpz_scan1"]
    fn mpz_scan1(a: Zc, start: mp_bitcnt_t) -> mp_bitcnt_t;

    #[link_name = "__gmpz_import"]
    fn mpz_import(
        r: Z,
        count: usize,
        order: c_int,
        size: usize,
        endian: c_int,
        nails: usize,
        op: *const c_void,
    );
    #[link_name = "__gmpz_export"]
    fn mpz_export(
        rop: *mut c_void,
        countp: *mut usize,
        order: c_int,
        size: usize,
        endian: c_int,
        nails: usize,
        a: Zc,
    ) -> *mut c_void;

    #[link_name = "__gmpz_urandomb"]
    fn mpz_urandomb(r: Z, state: R, n: mp_bitcnt_t);
    #[link_name = "__gmpz_urandomm"]
    fn mpz_urandomm(r: Z, state: R, n: Zc);
    #[link_name = "__gmp_randinit_mt"]
    fn gmp_randinit_mt(state: R);
    #[link_name = "__gmp_randseed"]
    fn gmp_randseed(state: R, seed: Zc);
    #[link_name = "__gmp_randseed_ui"]
    fn gmp_randseed_ui(state: R, seed: c_ulong);
    #[link_name = "__gmp_randclear"]
    fn gmp_randclear(state: R);
}

// shim.c: the layout checks, and the header's version for a test.
#[cfg(test)]
extern "C" {
    fn rhp_gmp_header_version() -> *const c_char;
}

/// `gmp_version`, as php reports it in `GMP_VERSION` and phpinfo.
pub fn version() -> &'static str {
    // SAFETY: a NUL-terminated static string in libgmp.
    unsafe { CStr::from_ptr(gmp_version) }
        .to_str()
        .unwrap_or("")
}

/// The version of the gmp.h rhp was compiled against.
#[cfg(test)]
pub fn header_version() -> &'static str {
    // SAFETY: a string literal in shim.c.
    unsafe { CStr::from_ptr(rhp_gmp_header_version()) }
        .to_str()
        .unwrap_or("")
}

/// An initialised `mpz_t`, cleared on drop.
pub struct Mpz(MpzStruct);

impl Default for Mpz {
    fn default() -> Self {
        Mpz::new()
    }
}

impl Drop for Mpz {
    fn drop(&mut self) {
        // SAFETY: initialised by `new`.
        unsafe { mpz_clear(&mut self.0) }
    }
}

impl Clone for Mpz {
    fn clone(&self) -> Self {
        let mut r = Mpz::new();
        r.set(self);
        r
    }
}

impl Mpz {
    /// 0.
    pub fn new() -> Mpz {
        let mut z = MpzStruct {
            alloc: 0,
            size: 0,
            d: std::ptr::null_mut(),
        };
        // SAFETY: mpz_init writes every field.
        unsafe { mpz_init(&mut z) };
        Mpz(z)
    }

    pub fn from_i64(v: i64) -> Mpz {
        let mut r = Mpz::new();
        // SAFETY: an initialised mpz.
        unsafe { mpz_set_si(r.ptr(), v) };
        r
    }

    pub fn from_u64(v: u64) -> Mpz {
        let mut r = Mpz::new();
        // SAFETY: an initialised mpz.
        unsafe { mpz_set_ui(r.ptr(), v) };
        r
    }

    fn ptr(&mut self) -> Z {
        &mut self.0
    }

    fn cptr(&self) -> Zc {
        &self.0
    }

    pub fn set(&mut self, a: &Mpz) {
        // SAFETY: initialised mpzs; `a` is not `self` (borrows).
        unsafe { mpz_set(self.ptr(), a.cptr()) }
    }

    /// `mpz_set_str`: false when `s` (up to its first NUL byte, as C reads
    /// it) is not a number in `base`. The value is unspecified then.
    pub fn set_str(&mut self, s: &[u8], base: i32) -> bool {
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        let mut buf = Vec::with_capacity(end + 1);
        buf.extend_from_slice(&s[..end]);
        buf.push(0);
        // SAFETY: a NUL-terminated buffer.
        unsafe { mpz_set_str(self.ptr(), buf.as_ptr().cast(), base) == 0 }
    }

    /// `mpz_get_str` into a buffer sized as php's `gmp_strval` sizes it.
    /// `base` is 2..=62 or -36..=-2.
    pub fn to_str(&self, base: i32) -> Vec<u8> {
        let mut len = self.sizeinbase(base.abs());
        if self.sgn() < 0 {
            len += 1;
        }
        let mut buf = vec![0u8; len + 1];
        // SAFETY: mpz_sizeinbase (+1 for a sign) is enough room, plus the NUL.
        unsafe { mpz_get_str(buf.as_mut_ptr().cast(), base, self.cptr()) };
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        buf.truncate(n);
        buf
    }

    pub fn sgn(&self) -> i32 {
        self.0.size.signum()
    }

    /// The number of limbs in use.
    pub fn limbs(&self) -> u64 {
        self.0.size.unsigned_abs() as u64
    }

    pub fn get_si(&self) -> i64 {
        // SAFETY: an initialised mpz.
        unsafe { mpz_get_si(self.cptr()) }
    }

    pub fn get_ui(&self) -> u64 {
        // SAFETY: an initialised mpz.
        unsafe { mpz_get_ui(self.cptr()) }
    }

    pub fn get_d(&self) -> f64 {
        // SAFETY: an initialised mpz.
        unsafe { mpz_get_d(self.cptr()) }
    }

    pub fn fits_slong(&self) -> bool {
        // SAFETY: an initialised mpz.
        unsafe { mpz_fits_slong_p(self.cptr()) != 0 }
    }

    pub fn sizeinbase(&self, base: i32) -> usize {
        // SAFETY: an initialised mpz; base 2..=62.
        unsafe { mpz_sizeinbase(self.cptr(), base) }
    }

    /// The number of bits of |self| (0 for 0).
    pub fn bits(&self) -> u64 {
        if self.sgn() == 0 {
            0
        } else {
            self.sizeinbase(2) as u64
        }
    }

    pub fn cmp(&self, b: &Mpz) -> i32 {
        // SAFETY: initialised mpzs.
        unsafe { mpz_cmp(self.cptr(), b.cptr()) }
    }

    pub fn cmp_si(&self, b: i64) -> i32 {
        // SAFETY: an initialised mpz.
        unsafe { mpz_cmp_si(self.cptr(), b) }
    }

    /// `f(r, a, b)` into a new number.
    pub fn binop(f: BinOp, a: &Mpz, b: &Mpz) -> Mpz {
        let mut r = Mpz::new();
        // SAFETY: initialised mpzs; libgmp allows any aliasing of inputs.
        unsafe { f(r.ptr(), a.cptr(), b.cptr()) };
        r
    }

    /// `f(r, a)` into a new number.
    pub fn unop(f: UnOp, a: &Mpz) -> Mpz {
        let mut r = Mpz::new();
        // SAFETY: initialised mpzs.
        unsafe { f(r.ptr(), a.cptr()) };
        r
    }

    /// `f(r, a, n)` into a new number.
    pub fn uiop(f: UiOp, a: &Mpz, n: u64) -> Mpz {
        let mut r = Mpz::new();
        // SAFETY: initialised mpzs.
        unsafe { f(r.ptr(), a.cptr(), n) };
        r
    }

    /// `f(q, r, a, b)`: a quotient and remainder.
    pub fn divqr(f: DivQr, a: &Mpz, b: &Mpz) -> (Mpz, Mpz) {
        let (mut q, mut r) = (Mpz::new(), Mpz::new());
        // SAFETY: initialised, distinct outputs.
        unsafe { f(q.ptr(), r.ptr(), a.cptr(), b.cptr()) };
        (q, r)
    }

    pub fn add_ui(&self, n: u64) -> Mpz {
        let mut r = Mpz::new();
        // SAFETY: initialised mpzs.
        unsafe { mpz_add_ui(r.ptr(), self.cptr(), n) };
        r
    }

    pub fn sqrtrem(&self) -> (Mpz, Mpz) {
        let (mut s, mut r) = (Mpz::new(), Mpz::new());
        // SAFETY: initialised, distinct outputs; the caller checked sgn >= 0.
        unsafe { mpz_sqrtrem(s.ptr(), r.ptr(), self.cptr()) };
        (s, r)
    }

    pub fn fac_ui(n: u64) -> Mpz {
        let mut r = Mpz::new();
        // SAFETY: an initialised mpz; the caller checked the size.
        unsafe { mpz_fac_ui(r.ptr(), n) };
        r
    }

    pub fn bin_ui(&self, k: u64) -> Mpz {
        let mut r = Mpz::new();
        // SAFETY: initialised mpzs; the caller checked the size.
        unsafe { mpz_bin_ui(r.ptr(), self.cptr(), k) };
        r
    }

    pub fn powm(&self, e: &Mpz, m: &Mpz) -> Mpz {
        let mut r = Mpz::new();
        // SAFETY: initialised mpzs; the caller checked m != 0 and e >= 0.
        unsafe { mpz_powm(r.ptr(), self.cptr(), e.cptr(), m.cptr()) };
        r
    }

    pub fn root(&self, n: u64) -> Mpz {
        let mut r = Mpz::new();
        // SAFETY: initialised mpzs; the caller checked n > 0 and the sign.
        unsafe { mpz_root(r.ptr(), self.cptr(), n) };
        r
    }

    pub fn rootrem(&self, n: u64) -> (Mpz, Mpz) {
        let (mut r, mut rem) = (Mpz::new(), Mpz::new());
        // SAFETY: as for `root`, distinct outputs.
        unsafe { mpz_rootrem(r.ptr(), rem.ptr(), self.cptr(), n) };
        (r, rem)
    }

    pub fn perfect_square(&self) -> bool {
        // SAFETY: an initialised mpz.
        unsafe { mpz_perfect_square_p(self.cptr()) != 0 }
    }

    pub fn perfect_power(&self) -> bool {
        // SAFETY: an initialised mpz.
        unsafe { mpz_perfect_power_p(self.cptr()) != 0 }
    }

    pub fn probab_prime(&self, reps: i32) -> i32 {
        // SAFETY: an initialised mpz.
        unsafe { mpz_probab_prime_p(self.cptr(), reps) }
    }

    pub fn gcdext(a: &Mpz, b: &Mpz) -> (Mpz, Mpz, Mpz) {
        let (mut g, mut s, mut t) = (Mpz::new(), Mpz::new(), Mpz::new());
        // SAFETY: initialised, distinct outputs.
        unsafe { mpz_gcdext(g.ptr(), s.ptr(), t.ptr(), a.cptr(), b.cptr()) };
        (g, s, t)
    }

    /// `mpz_invert`: `None` when there is no inverse.
    pub fn invert(&self, m: &Mpz) -> Option<Mpz> {
        let mut r = Mpz::new();
        // SAFETY: initialised mpzs; the caller checked m != 0.
        let ok = unsafe { mpz_invert(r.ptr(), self.cptr(), m.cptr()) };
        (ok != 0).then_some(r)
    }

    pub fn jacobi(&self, b: &Mpz) -> i32 {
        // SAFETY: initialised mpzs.
        unsafe { mpz_jacobi(self.cptr(), b.cptr()) }
    }

    pub fn setbit(&mut self, i: u64) {
        // SAFETY: an initialised mpz; the caller bounded `i`.
        unsafe { mpz_setbit(self.ptr(), i) }
    }

    pub fn clrbit(&mut self, i: u64) {
        // SAFETY: as for `setbit`.
        unsafe { mpz_clrbit(self.ptr(), i) }
    }

    pub fn tstbit(&self, i: u64) -> bool {
        // SAFETY: an initialised mpz.
        unsafe { mpz_tstbit(self.cptr(), i) != 0 }
    }

    pub fn popcount(&self) -> u64 {
        // SAFETY: an initialised mpz.
        unsafe { mpz_popcount(self.cptr()) }
    }

    pub fn hamdist(&self, b: &Mpz) -> u64 {
        // SAFETY: initialised mpzs.
        unsafe { mpz_hamdist(self.cptr(), b.cptr()) }
    }

    pub fn scan0(&self, start: u64) -> u64 {
        // SAFETY: an initialised mpz.
        unsafe { mpz_scan0(self.cptr(), start) }
    }

    pub fn scan1(&self, start: u64) -> u64 {
        // SAFETY: an initialised mpz.
        unsafe { mpz_scan1(self.cptr(), start) }
    }

    /// `mpz_import` of `data.len() / size` words.
    pub fn import(data: &[u8], order: i32, size: usize, endian: i32) -> Mpz {
        let mut r = Mpz::new();
        // SAFETY: `data` holds count * size bytes.
        unsafe {
            mpz_import(
                r.ptr(),
                data.len() / size,
                order,
                size,
                endian,
                0,
                data.as_ptr().cast(),
            )
        };
        r
    }

    /// `mpz_export` into `count * size` bytes, `count` as php computes it.
    pub fn export(&self, order: i32, size: usize, endian: i32, count: usize) -> Vec<u8> {
        let mut out = vec![0u8; count * size];
        let mut written = 0usize;
        // SAFETY: `out` has room for every word of the number.
        unsafe {
            mpz_export(
                out.as_mut_ptr().cast(),
                &mut written,
                order,
                size,
                endian,
                0,
                self.cptr(),
            )
        };
        out
    }

    pub fn urandomb(state: &mut RandState, bits: u64) -> Mpz {
        let mut r = Mpz::new();
        // SAFETY: an initialised state and mpz.
        unsafe { mpz_urandomb(r.ptr(), state.ptr(), bits) };
        r
    }

    pub fn urandomm(state: &mut RandState, n: &Mpz) -> Mpz {
        let mut r = Mpz::new();
        // SAFETY: an initialised state and mpzs; the caller checked n > 0.
        unsafe { mpz_urandomm(r.ptr(), state.ptr(), n.cptr()) };
        r
    }
}

/// A Mersenne-twister `gmp_randstate_t` (`gmp_randinit_mt`), cleared on
/// drop.
pub struct RandState(Box<RandStruct>);

impl RandState {
    pub fn new_mt() -> RandState {
        let mut s = Box::new(RandStruct([0; 32]));
        // SAFETY: gmp_randinit_mt initialises the struct.
        unsafe { gmp_randinit_mt(&mut *s) };
        RandState(s)
    }

    fn ptr(&mut self) -> R {
        &mut *self.0
    }

    pub fn seed(&mut self, seed: &Mpz) {
        // SAFETY: an initialised state and mpz.
        unsafe { gmp_randseed(self.ptr(), seed.cptr()) }
    }

    pub fn seed_ui(&mut self, seed: u64) {
        // SAFETY: an initialised state.
        unsafe { gmp_randseed_ui(self.ptr(), seed) }
    }
}

impl Drop for RandState {
    fn drop(&mut self) {
        // SAFETY: initialised by `new_mt`.
        unsafe { gmp_randclear(self.ptr()) }
    }
}

#[cfg(test)]
mod tests;
