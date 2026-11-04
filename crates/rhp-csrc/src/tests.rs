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
    if cfg!(feature = "crypt") {
        want.push("librhp_csrc_crypt.a");
    }
    if cfg!(feature = "timelib") {
        want.push("librhp_csrc_timelib.a");
    }
    want.push("librhp_csrc_zend.a");
    assert_eq!(seen, want);
    assert!(bad.is_empty(), "unprefixed symbols:\n{}", bad.join("\n"));
}
