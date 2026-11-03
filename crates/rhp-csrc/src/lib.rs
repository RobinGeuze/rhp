//! C that rhp compiles from source and links statically: php-src's own C at
//! the pinned tag (`vendor/php-src`, written by `tools/sync-php-src-c.sh`)
//! and frozen libraries whose only release is the one php links
//! (`vendor/argon2`). All php-src C shares one Zend shim, `shim/zend_shim.h`.
//!
//! One cargo feature per library. Each module wraps its C in safe functions
//! that take what php passes and return what php's C returns. Every global
//! symbol the C defines gets an rhp_ prefix at compile time
//! (`glue/prefix_<lib>.h`), so other static C libraries can't clash with it.

#[cfg(feature = "argon2")]
pub mod argon2;
#[cfg(feature = "crypt")]
pub mod crypt;

#[cfg(test)]
mod tests {
    use std::path::Path;
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
        assert_eq!(seen, want);
        assert!(bad.is_empty(), "unprefixed symbols:\n{}", bad.join("\n"));
    }
}
