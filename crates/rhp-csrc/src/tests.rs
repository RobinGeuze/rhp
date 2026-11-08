use std::ffi::c_void;
use std::path::Path;

#[test]
fn emalloc_blocks_round_trip() {
    for size in [0usize, 1, 15, 16, 4096, 1 << 20] {
        let p = super::rhp_csrc_emalloc(size) as *mut u8;
        assert_eq!(p as usize % 16, 0);
        // SAFETY: the block holds at least `size` bytes.
        unsafe {
            std::ptr::write_bytes(p, 0xab, size);
            let q = super::rhp_csrc_erealloc(p as *mut c_void, size * 2 + 1) as *mut u8;
            assert_eq!(q as usize % 16, 0);
            assert!((0..size).all(|i| *q.add(i) == 0xab));
            super::rhp_csrc_efree(q as *mut c_void);
        }
    }
    // SAFETY: NULL is a no-op, as in php.
    unsafe { super::rhp_csrc_efree(std::ptr::null_mut()) };
}

#[test]
fn emalloc_block_becomes_a_vec() {
    for (size, offset, len) in [(1usize, 0usize, 1usize), (64, 24, 40), (1 << 20, 24, 1000)] {
        let p = super::rhp_csrc_emalloc(size) as *mut u8;
        // SAFETY: the block holds `size` bytes, `offset + len` of them
        // written; it is not used after emalloc_into_vec.
        let v = unsafe {
            for i in 0..offset + len {
                *p.add(i) = i as u8;
            }
            super::emalloc_into_vec(p as *mut c_void, offset, len)
        };
        assert_eq!(v.len(), len);
        assert!(v.iter().enumerate().all(|(i, &b)| b == (offset + i) as u8));
    }
}

#[test]
fn fallible_realloc_stops_at_the_limit() {
    use rhp_values::mem;
    std::thread::spawn(|| {
        mem::begin_request();
        mem::force_limit(4 * mem::CHUNK_SIZE);
        let p = super::rhp_csrc_emalloc(16);
        // SAFETY: p came from rhp_csrc_emalloc.
        unsafe {
            let q = super::rhp_csrc_safe_ferealloc(p, 1, 1 << 20, 24);
            assert!(!q.is_null());
            let r = super::rhp_csrc_safe_ferealloc(q, 1, 64 << 20, 24);
            assert!(r.is_null());
            assert!(mem::state().attention());
            assert!(super::rhp_csrc_safe_ferealloc(q, usize::MAX, 2, 0).is_null());
            super::rhp_csrc_efree(q);
        }
        assert_eq!(super::rhp_csrc_safe_address(usize::MAX, 2, 0), usize::MAX);
        mem::end_request();
    })
    .join()
    .unwrap();
}
use std::process::Command;

/// Every global symbol the C archives define carries the rhp_ prefix
/// (glue/prefix_<lib>.h), so another static C library can't clash with
/// ours or bind to it. Hidden symbols count: visibility doesn't matter in
/// a static link.
#[test]
fn archives_define_only_prefixed_symbols() {
    let out = Path::new(env!("OUT_DIR"));
    let mut seen = Vec::new();
    let mut bad = Vec::new();
    for entry in std::fs::read_dir(out).unwrap() {
        let path = entry.unwrap().path();
        if path.extension() != Some("a".as_ref()) {
            continue;
        }
        seen.push(path.file_name().unwrap().to_string_lossy().into_owned());
        // -P: "name type [value size]", plus "archive[member]:" lines.
        let nm = Command::new("nm")
            .args(["-g", "-P"])
            .arg(&path)
            .output()
            .expect("this test runs nm (binutils or Xcode command line tools)");
        assert!(nm.status.success(), "nm {}: {:?}", path.display(), nm);
        for line in String::from_utf8_lossy(&nm.stdout).lines() {
            let mut fields = line.split_whitespace();
            let (Some(name), Some(ty)) = (fields.next(), fields.next()) else {
                continue;
            };
            if matches!(ty, "U" | "w" | "v") {
                continue;
            }
            let name = if cfg!(target_vendor = "apple") {
                name.strip_prefix('_').unwrap_or(name)
            } else {
                name
            };
            if !name.starts_with("rhp_") {
                bad.push(format!("{}: {ty} {name}", path.display()));
            }
        }
    }
    seen.sort();
    let mut want = Vec::new();
    if cfg!(feature = "argon2") {
        want.push("librhp_csrc_argon2.a");
    }
    if cfg!(feature = "avifinfo") {
        want.push("librhp_csrc_avifinfo.a");
    }
    if cfg!(feature = "crypt") {
        want.push("librhp_csrc_crypt.a");
    }
    if cfg!(feature = "fileinfo") {
        want.push("librhp_csrc_fileinfo.a");
    }
    if cfg!(feature = "lexbor") {
        want.push("librhp_csrc_lexbor.a");
    }
    if cfg!(feature = "timelib") {
        want.push("librhp_csrc_timelib.a");
    }
    want.push("librhp_csrc_zend.a");
    assert_eq!(seen, want);
    assert!(bad.is_empty(), "unprefixed symbols:\n{}", bad.join("\n"));
}

mod php_printf {
    use std::ffi::{c_char, c_int};

    extern "C" {
        fn rhp_csrc_snprintf(buf: *mut c_char, len: usize, format: *const c_char, ...) -> c_int;
        fn rhp_csrc_spprintf(
            pbuf: *mut *mut c_char,
            max_len: usize,
            format: *const c_char,
            ...
        ) -> usize;
    }

    fn sn(f: impl FnOnce(*mut c_char, usize) -> c_int) -> (String, c_int) {
        let mut buf = [0 as c_char; 64];
        let n = f(buf.as_mut_ptr(), buf.len());
        let s = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
        (s.to_string_lossy().into_owned(), n)
    }

    /// main/snprintf.c's conversions, where they differ from the C
    /// library's: exponents without padding, %g as zend_gcvt, NAN/INF, the
    /// 0 flag kept with a precision, "(null)".
    #[test]
    fn prints_like_php() {
        unsafe {
            let cases: Vec<((String, c_int), &str)> = vec![
                (
                    sn(|b, l| rhp_csrc_snprintf(b, l, c"%e".as_ptr(), 1.5f64)),
                    "1.500000e+0",
                ),
                (
                    sn(|b, l| rhp_csrc_snprintf(b, l, c"%g|%g".as_ptr(), 1e-5f64, 1e20f64)),
                    "1.0e-5|1.0e+20",
                ),
                (
                    sn(|b, l| rhp_csrc_snprintf(b, l, c"%.2f|%f".as_ptr(), 2.345f64, f64::NAN)),
                    "2.35|NAN",
                ),
                (
                    sn(|b, l| {
                        rhp_csrc_snprintf(b, l, c"%#x|%#x|%08.4x".as_ptr(), 0u32, 255u32, 0xabcu32)
                    }),
                    "0|0xff|00000abc",
                ),
                (
                    sn(|b, l| {
                        rhp_csrc_snprintf(
                            b,
                            l,
                            c"%-5s|%.2s|%s".as_ptr(),
                            c"ab".as_ptr(),
                            c"xyz".as_ptr(),
                            std::ptr::null::<c_char>(),
                        )
                    }),
                    "ab   |xy|(null)",
                ),
                (
                    sn(|b, l| {
                        rhp_csrc_snprintf(b, l, c"%lld|%zu|%+d".as_ptr(), -5i64, 7usize, 3 as c_int)
                    }),
                    "-5|7|+3",
                ),
            ];
            for ((got, n), want) in cases {
                assert_eq!(got, want);
                assert_eq!(n as usize, want.len());
            }
            // Cut to the buffer, the full length returned.
            let mut small = [0 as c_char; 4];
            let n = rhp_csrc_snprintf(small.as_mut_ptr(), 4, c"%s".as_ptr(), c"abcdef".as_ptr());
            assert_eq!(n, 6);
            assert_eq!(std::ffi::CStr::from_ptr(small.as_ptr()).to_bytes(), b"abc");
            let mut p = std::ptr::null_mut();
            let n = rhp_csrc_spprintf(&mut p, 0, c"%e|%d".as_ptr(), f64::INFINITY, 42 as c_int);
            assert_eq!(std::ffi::CStr::from_ptr(p).to_bytes(), b"inf|42");
            assert_eq!(n, 6);
            super::super::rhp_csrc_efree(p.cast());
        }
    }
}
