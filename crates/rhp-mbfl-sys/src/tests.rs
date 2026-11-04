use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use super::*;

// Counts this test binary's allocations, as rhp's binaries do.
#[global_allocator]
static ALLOC: rhp_values::mem::CountingAlloc = rhp_values::mem::CountingAlloc;

/// The global symbols the archives in `dir` define, with the archive each
/// comes from.
fn defined_symbols(dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension() != Some("a".as_ref()) {
            continue;
        }
        // -P: "name type [value size]", plus "archive[member]:" lines.
        let nm = Command::new("nm")
            .args(["-g", "-P"])
            .arg(&path)
            .output()
            .expect("this test runs nm (binutils or Xcode command line tools)");
        assert!(nm.status.success(), "nm {}: {:?}", path.display(), nm);
        let archive = path.file_name().unwrap().to_string_lossy().into_owned();
        for line in String::from_utf8_lossy(&nm.stdout).lines() {
            let mut fields = line.split_whitespace();
            let (Some(name), Some(ty)) = (fields.next(), fields.next()) else {
                continue;
            };
            if name.ends_with(':') || matches!(ty, "U" | "w" | "v") {
                continue;
            }
            let name = if cfg!(target_vendor = "apple") {
                name.strip_prefix('_').unwrap_or(name)
            } else {
                name
            };
            out.push((archive.clone(), name.to_string()));
        }
    }
    out
}

/// Every global symbol the archive defines carries the rhp_ prefix
/// (glue/prefix_mbfl.h), so another static C library can't clash with it or
/// bind to it. Hidden symbols count: visibility doesn't matter in a static
/// link.
#[test]
fn archive_defines_only_prefixed_symbols() {
    let syms = defined_symbols(Path::new(env!("OUT_DIR")));
    let archives: BTreeSet<_> = syms.iter().map(|s| s.0.as_str()).collect();
    assert_eq!(archives, BTreeSet::from(["librhp_mbfl.a"]));
    let bad: BTreeSet<_> = syms
        .iter()
        .filter(|s| !s.1.starts_with("rhp_"))
        .map(|s| format!("#define {0} rhp_{0}", s.1))
        .collect();
    assert!(
        bad.is_empty(),
        "unprefixed symbols; add to glue/prefix_mbfl.h (sorted):\n{}",
        bad.into_iter().collect::<Vec<_>>().join("\n")
    );
}

/// rhp-csrc's shim defines zend_empty_string and the allocator once; no
/// symbol is defined in both crates' archives.
#[test]
fn no_symbol_defined_twice_with_rhp_csrc() {
    let ours: BTreeSet<_> = defined_symbols(Path::new(env!("OUT_DIR")))
        .into_iter()
        .map(|s| s.1)
        .collect();
    let csrc = defined_symbols(Path::new(env!("RHP_CSRC_OUT_DIR")));
    assert!(csrc.iter().any(|s| s.1 == "rhp_zend_empty_string"));
    let twice: Vec<_> = csrc.iter().filter(|s| ours.contains(&s.1)).collect();
    assert!(twice.is_empty(), "defined twice: {twice:?}");
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn names_aliases_and_list() {
    let sjis = Encoding::by_name(b"shift_jis").unwrap();
    assert_eq!(sjis.name(), b"SJIS");
    assert_eq!(Encoding::by_name(b"UTF-8\0junk"), Some(encodings::utf8()));
    assert_eq!(Encoding::by_name_ex(b"UTF-8\0junk"), None);
    assert!(Encoding::by_name(b"nope").is_none());
    let names: Vec<_> = Encoding::supported().map(|e| e.name()).collect();
    assert_eq!(names.first(), Some(&&b"BASE64"[..]));
    assert_eq!(names.last(), Some(&&b"CP50222"[..]));
    assert!(encodings::utf8().aliases().any(|a| a == b"utf8"));
    assert_eq!(encodings::utf8().no(), no::UTF8);
    let ucs2 = Encoding::by_name(b"UCS-2LE").unwrap();
    assert_eq!(ucs2.flag() & ENCTYPE_WCS2, ENCTYPE_WCS2);
    assert_eq!(encodings::utf16le().flag() & ENCTYPE_WCS2, 0);
}

#[test]
fn conversion_case_and_cut() {
    let sjis = Encoding::by_name(b"SJIS").unwrap();
    // 0x81 0x60 is U+301C in php's SJIS; 0x87 0x40 is unmapped there.
    let (out, errors) = fast_convert(
        b"\x81\x60\x87\x40",
        sjis,
        encodings::utf8(),
        '?' as u32,
        illegal_mode::CHAR,
    );
    assert_eq!(hex(&out), "e3809c3f");
    assert_eq!(errors, 1);
    let title = convert_case(
        case_mode::TITLE,
        "\u{345}".as_bytes(),
        encodings::utf8(),
        encodings::utf8(),
        illegal_mode::CHAR,
        '?' as u32,
    );
    assert_eq!(hex(&title), "ce99");
    // UTF-16's cut returns the static empty string for an empty result.
    assert_eq!(encodings::utf16be().cut(b"\x00a", 2, 4), Some(Vec::new()));
    assert_eq!(
        encodings::utf16be().cut(b"\x00a\x00b", 1, 2),
        Some(b"\x00a".to_vec())
    );
    assert_eq!(
        strcut("日本語".as_bytes(), encodings::utf8(), 1, 4),
        Some("日".as_bytes().to_vec())
    );
    let mut buf = ConvertBuf::new(4, '?' as u32, illegal_mode::CHAR);
    encodings::utf8().from_wchar(&[0x65e5, 0x672c], &mut buf, false);
    assert!(buf.append(b"..."));
    assert_eq!(buf.result_raw(), "日本...".as_bytes());
    let mut state = 0;
    let mut input = &b"a\xffb"[..];
    let mut out = [0u32; 8];
    let n = encodings::utf8().to_wchar(&mut input, &mut out, &mut state);
    assert_eq!(&out[..n], &[0x61, BAD_INPUT, 0x62]);
    assert!(input.is_empty());
    assert_eq!(eaw_table()[0], [0x1100, 0x115f]);
    assert_eq!(rare_cp_bitvec().len(), 2048);
}

/// libmbfl's buffers grow through the fallible realloc: past memory_limit
/// the conversion stops early and the error waits for the VM.
#[test]
fn growth_stops_at_memory_limit() {
    use rhp_values::mem;
    std::thread::spawn(|| {
        mem::begin_request();
        mem::force_limit(4 * mem::CHUNK_SIZE);
        let input = vec![b'a'; 2 * 1024 * 1024];
        let (out, _) = fast_convert(
            &input,
            encodings::ascii(),
            Encoding::by_name(b"UTF-32").unwrap(),
            '?' as u32,
            illegal_mode::CHAR,
        );
        assert!(out.len() < 4 * input.len());
        assert!(mem::state().attention());
        mem::end_request();
    })
    .join()
    .unwrap();
}

/// A result is held once: the Vec takes over the C's buffer.
#[test]
fn results_are_not_copied() {
    use rhp_values::mem;
    std::thread::spawn(|| {
        mem::begin_request();
        let input = vec![b'a'; 4 * 1024 * 1024];
        mem::reset_peak();
        let base = mem::usage(false);
        let (out, _) = fast_convert(
            &input,
            encodings::ascii(),
            encodings::utf16le(),
            '?' as u32,
            illegal_mode::CHAR,
        );
        let peak = mem::peak_usage(false) - base;
        assert_eq!(out.len(), 2 * input.len());
        assert!(peak < out.len() + out.len() / 4, "peak {peak}");
        mem::end_request();
    })
    .join()
    .unwrap();
}
