use super::*;

fn compile(body: &str, options: u32) -> Regex {
    let mut issue = None;
    let re = Regex::compile(body.as_bytes(), options, 0, &mut issue)
        .ok()
        .unwrap();
    assert!(issue.is_none());
    re
}

fn all(re: &Regex, subject: &[u8]) -> Result<Vec<Ovector>, i32> {
    let mut out = Vec::new();
    re.find_all(subject, 0, usize::MAX, &mut out).map(|()| out)
}

/// This test binary is linked straight from the crate, not through the
/// rlib, so it shows whether the build helper's static link holds.
#[test]
fn pcre2_is_linked_statically() {
    if env!("RHP_PCRE2_LINK_MODE") != "static" {
        return;
    }
    let exe = std::env::current_exe().unwrap();
    let tool = if cfg!(target_os = "macos") {
        ("otool", vec!["-L"])
    } else {
        ("ldd", vec![])
    };
    let Ok(out) = std::process::Command::new(tool.0)
        .args(&tool.1)
        .arg(&exe)
        .output()
    else {
        return;
    };
    let deps = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{} failed: {deps}", tool.0);
    assert!(!deps.contains("libpcre2"), "{deps}");
}

#[test]
fn header_and_library_agree() {
    let v = version();
    assert!(v.starts_with(&format!("{MAJOR}.{MINOR} ")), "{v}");
    assert!(!unicode_version().is_empty());
}

#[test]
fn jit_follows_pcre_jit() {
    reset();
    if !jit_supported() {
        return;
    }
    assert!(jit_target().is_some());
    assert!(compile("a+b", 0).jit);
    ini_update(b"pcre.jit", b"off");
    assert!(!compile("a+b", 0).jit);
    ini_update(b"pcre.jit", b"1");
    assert!(compile("a+b", 0).jit);
}

#[test]
fn backtrack_and_recursion_limits() {
    reset();
    ini_update(b"pcre.jit", b"0");
    ini_update(b"pcre.backtrack_limit", b"2");
    let re = compile(r".*\p{N}", 0);
    assert_eq!(all(&re, b"0123456789").err(), Some(PCRE2_ERROR_MATCHLIMIT));
    ini_update(b"pcre.backtrack_limit", b"1000000");
    assert_eq!(all(&re, b"0123456789").unwrap().len(), 1);

    ini_update(b"pcre.recursion_limit", b"2");
    let re = compile(r"\p{Ll}(\p{L}((\p{Ll}\p{Ll})))", 0);
    assert_eq!(all(&re, b"aeiou").err(), Some(PCRE2_ERROR_DEPTHLIMIT));
    reset();
    assert_eq!(all(&re, b"aeiou").unwrap().len(), 1);
}

#[test]
fn jit_stack_is_php_sized() {
    reset();
    if !jit_supported() {
        return;
    }
    // php's 192 KiB JIT stack runs out on this one (errors05.phpt).
    let re = compile("^(foo)+$", 0);
    let subject = b"foo".repeat(1024 * 8192);
    assert_eq!(
        re.find_at(&subject, 0).err(),
        Some(PCRE2_ERROR_JIT_STACKLIMIT)
    );
}

#[test]
fn settings_are_per_thread() {
    reset();
    ini_update(b"pcre.jit", b"0");
    ini_update(b"pcre.backtrack_limit", b"2");
    let other = std::thread::spawn(|| {
        let re = compile(r".*\p{N}", 0);
        all(&re, b"0123456789").map(|v| v.len())
    });
    assert_eq!(other.join().unwrap(), Ok(1));
    reset();
}

#[test]
fn full_cache_drops_its_oldest_unused_eighth() {
    reset();
    let held = Rc::new(compile("x", 0));
    cache_put(b"/0/".to_vec(), held.clone());
    for i in 1..CACHE_SIZE {
        cache_put(format!("/{i}/").into_bytes(), Rc::new(compile("x", 0)));
    }
    cache_put(b"/new/".to_vec(), Rc::new(compile("x", 0)));
    assert!(cache_get(b"/0/").is_some(), "in use, so kept");
    assert!(cache_get(b"/1/").is_none());
    assert!(cache_get(b"/512/").is_none());
    assert!(cache_get(b"/513/").is_some());
    assert!(cache_get(b"/new/").is_some());
    STATE.with(|s| assert_eq!(s.borrow().cache.len(), CACHE_SIZE - CACHE_SIZE / 8 + 1));
    reset();
}

#[test]
fn pattern_iterates_like_the_pcre2_crate() {
    let p = Pattern::new("a*", 0).unwrap();
    // An empty match right after a match is skipped.
    assert_eq!(
        p.captures_all(b"baab"),
        vec![vec![Some((0, 0))], vec![Some((1, 3))], vec![Some((4, 4))]]
    );
    let p = Pattern::new("^a.c$", DOTALL).unwrap();
    assert_eq!(p.is_match(b"a\nc"), Ok(true));
    assert!(Pattern::new("(", 0).is_err());
}
