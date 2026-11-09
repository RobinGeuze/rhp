//! The compiled-script cache across requests on one thread: a script is
//! reused while its file is unchanged, revalidated as php's settings say,
//! never shares per-request state, and preloaded declarations are fresh in
//! every request.
//!
//! The cache is per thread, so each test runs on a thread of its own.

use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use rhp_sapi::{CaptureSapi, Outcome, Request, RequestInfo, Script};

// The engine's memory accounting (memory_get_usage(), memory_limit).
#[global_allocator]
static ALLOC: rhp_sapi::CountingAlloc = rhp_sapi::CountingAlloc;

/// One request for `script` at `time` (seconds since the epoch), with
/// opcache on and `ini` on top; returns the body.
fn request(script: &Path, ini: &[(&str, &str)], time: u64) -> String {
    let (outcome, body) = run(script, ini, time);
    assert_eq!(outcome, Outcome::Exit(0));
    body
}

/// [`request`], with its outcome.
fn run(script: &Path, ini: &[(&str, &str)], time: u64) -> (Outcome, String) {
    let mut settings = vec![
        ("opcache.enable".to_string(), "1".to_string()),
        ("display_errors".to_string(), "1".to_string()),
    ];
    settings.extend(ini.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    let req = Request {
        ini: settings,
        info: Some(RequestInfo {
            method: Some(b"GET".to_vec()),
            response_code: 200,
            request_time: time as f64,
            ..Default::default()
        }),
        ..Request::new(Script::File(script.to_string_lossy().into_owned()))
    };
    let (outcome, cap) = CaptureSapi::web("embed-test").run(req);
    (outcome, String::from_utf8_lossy(&cap.stdout).into_owned())
}

/// A scratch directory of PHP files, removed when dropped.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("rhp-opcache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Dir(d.canonicalize().unwrap())
    }

    /// Write `name` with modification time `mtime`.
    fn file(&self, name: &str, src: &str, mtime: u64) -> PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, src).unwrap();
        let f = std::fs::File::options().write(true).open(&p).unwrap();
        f.set_modified(UNIX_EPOCH + Duration::from_secs(mtime))
            .unwrap();
        p
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn on_own_thread(f: impl FnOnce() + Send + 'static) {
    std::thread::spawn(f).join().unwrap();
}

const STATS: &str = r#"
$s = opcache_get_status(false)['opcache_statistics'];
echo "hits={$s['hits']} misses={$s['misses']} scripts={$s['num_cached_scripts']}\n";
"#;

#[test]
fn cached_scripts_keep_per_request_state_fresh() {
    on_own_thread(|| {
        let d = Dir::new("fresh");
        d.file(
            "lib.php",
            r#"<?php
            function counter() { static $n = 0; return ++$n; }
            class C { public static $n = 0; const K = 'k'; }
            "#,
            500,
        );
        let main = d.file(
            "main.php",
            &format!(
                r#"<?php
                var_dump(defined('MARK'), isset($GLOBALS['mark']), function_exists('counter'));
                require __DIR__ . '/lib.php';
                define('MARK', 1);
                $mark = 1;
                C::$n++;
                echo counter(), counter(), ' ', C::$n, ' ', C::K, "\n";
                {STATS}"#
            ),
            500,
        );
        let fresh = "bool(false)\nbool(false)\nbool(false)\n12 1 k\n";
        assert_eq!(
            request(&main, &[], 1000),
            format!("{fresh}hits=0 misses=2 scripts=2\n")
        );
        assert_eq!(
            request(&main, &[], 1001),
            format!("{fresh}hits=2 misses=2 scripts=2\n")
        );
        assert_eq!(
            request(&main, &[], 1002),
            format!("{fresh}hits=4 misses=2 scripts=2\n")
        );
    });
}

#[test]
fn a_changed_file_is_recompiled() {
    on_own_thread(|| {
        let d = Dir::new("changed");
        let lib = d.file("lib.php", "<?php return 'v1';", 500);
        let main = d.file(
            "main.php",
            "<?php echo include __DIR__ . '/lib.php', \"\\n\";",
            500,
        );
        let ini = [("opcache.revalidate_freq", "0")];
        assert_eq!(request(&main, &ini, 1000), "v1\n");
        assert_eq!(request(&main, &ini, 1001), "v1\n");
        d.file("lib.php", "<?php return 'v2';", 600);
        assert_eq!(request(&main, &ini, 1002), "v2\n");
        // Within one request too: the script rewrites the file between
        // two includes.
        let twice = d.file(
            "twice.php",
            &format!(
                r#"<?php
                $f = '{}';
                file_put_contents($f, '<?php return "v3";');
                touch($f, 700);
                echo include $f;
                file_put_contents($f, '<?php return "v4";');
                touch($f, 800);
                echo include $f, "\n";"#,
                lib.display()
            ),
            500,
        );
        assert_eq!(request(&twice, &ini, 1003), "v3v4\n");
    });
}

#[test]
fn revalidate_freq_defers_the_check_like_php() {
    on_own_thread(|| {
        let d = Dir::new("freq");
        d.file("lib.php", "<?php return 'v1';", 500);
        let main = d.file(
            "main.php",
            "<?php echo include __DIR__ . '/lib.php', \"\\n\";",
            500,
        );
        let ini = [("opcache.revalidate_freq", "2")];
        assert_eq!(request(&main, &ini, 1000), "v1\n");
        d.file("lib.php", "<?php return 'v2';", 600);
        // Checked at 1000, so not stat'ed again before 1002 (php's
        // `revalidate` is request time + revalidate_freq).
        assert_eq!(request(&main, &ini, 1001), "v1\n");
        assert_eq!(request(&main, &ini, 1002), "v1\n");
        assert_eq!(request(&main, &ini, 1003), "v2\n");
    });
}

#[test]
fn without_timestamp_validation_only_invalidation_reloads() {
    on_own_thread(|| {
        let d = Dir::new("novalidate");
        let lib = d.file("lib.php", "<?php return 'v1';", 500);
        let main = d.file(
            "main.php",
            "<?php echo include __DIR__ . '/lib.php', \"\\n\";",
            500,
        );
        let ini = [("opcache.validate_timestamps", "0")];
        assert_eq!(request(&main, &ini, 1000), "v1\n");
        d.file("lib.php", "<?php return 'v2';", 600);
        assert_eq!(request(&main, &ini, 1001), "v1\n");
        let inv = d.file(
            "inv.php",
            &format!(
                "<?php var_dump(opcache_invalidate('{}', true)); echo include '{}', \"\\n\";",
                lib.display(),
                lib.display()
            ),
            500,
        );
        assert_eq!(request(&inv, &ini, 1002), "bool(true)\nv2\n");
        assert_eq!(request(&main, &ini, 1003), "v2\n");
    });
}

#[test]
fn a_just_written_file_is_not_cached() {
    on_own_thread(|| {
        let d = Dir::new("fup");
        let fresh = d.file("fresh.php", "<?php return 1;", 999);
        let old = d.file("old.php", "<?php return 1;", 500);
        let main = d.file(
            "main.php",
            &format!(
                "<?php include '{f}'; include '{o}'; var_dump(opcache_is_script_cached('{f}'), opcache_is_script_cached('{o}'));",
                f = fresh.display(),
                o = old.display()
            ),
            500,
        );
        // php's default file_update_protection of 2 seconds.
        assert_eq!(request(&main, &[], 1000), "bool(false)\nbool(true)\n");
        let ini = [("opcache.file_update_protection", "0")];
        assert_eq!(request(&main, &ini, 1001), "bool(true)\nbool(true)\n");
    });
}

#[test]
fn opcache_reset_restarts_at_the_next_request() {
    on_own_thread(|| {
        let d = Dir::new("reset");
        d.file("lib.php", "<?php return 1;", 500);
        let main = d.file(
            "main.php",
            &format!(
                r#"<?php
                include __DIR__ . '/lib.php';
                $st = opcache_get_status(false);
                echo $st['opcache_statistics']['manual_restarts'], ' ', (int) $st['restart_pending'], "\n";
                if (!$st['opcache_statistics']['manual_restarts']) var_dump(opcache_reset());
                {STATS}"#
            ),
            500,
        );
        assert_eq!(
            request(&main, &[], 1000),
            "0 0\nbool(true)\nhits=0 misses=2 scripts=2\n"
        );
        // Restarting empties the cache and its counters.
        assert_eq!(
            request(&main, &[], 1001),
            "1 0\nhits=0 misses=2 scripts=2\n"
        );
    });
}

#[test]
fn preloaded_declarations_are_fresh_in_every_request() {
    on_own_thread(|| {
        let d = Dir::new("preload");
        let pre = d.file(
            "preload.php",
            r#"<?php
            function pf() { static $n = 0; return ++$n; }
            class PC { public static $n = 0; }
            $g = 1;
            define('PRE', 1);
            "#,
            500,
        );
        let main = d.file(
            "main.php",
            r#"<?php
            var_dump(function_exists('pf'), class_exists('PC', false), isset($g), defined('PRE'));
            PC::$n++;
            echo pf(), pf(), ' ', PC::$n, "\n";
            echo implode(',', opcache_get_status(false)['preload_statistics']['functions']), "\n";
            "#,
            500,
        );
        let path = pre.to_string_lossy().into_owned();
        let ini = [("opcache.preload", path.as_str())];
        let want = "bool(true)\nbool(true)\nbool(false)\nbool(false)\n12 1\npf\n";
        assert_eq!(request(&main, &ini, 1000), want);
        assert_eq!(request(&main, &ini, 1001), want);
    });
}

#[test]
fn a_failed_preload_fails_every_request_on_the_thread() {
    on_own_thread(|| {
        let d = Dir::new("preload-fail");
        let pre = d.file("preload.php", "<?php throw new Exception('no');", 500);
        let main = d.file("main.php", "<?php echo 'ran';", 500);
        let path = pre.to_string_lossy().into_owned();
        let ini = [("opcache.preload", path.as_str())];
        for time in [1000, 1001] {
            let (outcome, body) = run(&main, &ini, time);
            assert_eq!(outcome, Outcome::Exit(1));
            assert!(!body.contains("ran"), "{body}");
        }
    });
}

/// php starts an included file's statics fresh on every include, also when
/// the include runs the same cached code (here through one unit, since the
/// file declares no closures and no classes).
#[test]
fn a_cached_include_starts_its_statics_fresh() {
    on_own_thread(|| {
        let d = Dir::new("statics");
        d.file("st.php", "<?php\nstatic $s = 0;\necho ++$s;\n", 500);
        let main = d.file(
            "main.php",
            r#"<?php
            for ($i = 0; $i < 3; $i++) include __DIR__ . '/st.php';
            echo ' ';
            for ($i = 0; $i < 3; $i++) echo eval('static $e = 0; return ++$e;');
            "#,
            500,
        );
        assert_eq!(request(&main, &[], 1000), "111 111");
        assert_eq!(request(&main, &[], 1001), "111 111");
    });
}

/// php compiles a script into the request heap without opcache, and into
/// shared memory with it: a cached program is never the request's memory,
/// neither when compiled nor when discarded.
#[test]
fn cached_programs_are_outside_the_request_memory() {
    on_own_thread(|| {
        let d = Dir::new("memory");
        let body: String = (0..2000)
            .map(|i| format!("    $v{i} = 'value {i}' . PHP_EOL;\n"))
            .collect();
        d.file(
            "lib.php",
            &format!("<?php\nfunction f() {{\n{body}}}\n"),
            500,
        );
        let main = d.file(
            "main.php",
            r#"<?php
            $before = memory_get_usage();
            include __DIR__ . '/lib.php';
            echo memory_get_usage() - $before;
            "#,
            500,
        );
        let discard = d.file(
            "discard.php",
            r#"<?php
            $before = memory_get_usage();
            opcache_invalidate(__DIR__ . '/lib.php', true);
            echo memory_get_usage() - $before;
            "#,
            500,
        );
        let grew = |script: &Path, ini: &[(&str, &str)], time| -> i64 {
            request(script, ini, time).parse().unwrap()
        };
        let uncached = grew(&main, &[("opcache.enable", "0")], 1000);
        let miss = grew(&main, &[], 1001);
        let hit = grew(&main, &[], 1002);
        assert!(uncached > hit + 100_000, "uncached {uncached}, hit {hit}");
        assert!((miss - hit).abs() < 4096, "miss {miss}, hit {hit}");
        // The include runs the cached code as it is (php: 352 bytes).
        assert!(hit < 4096, "a cache hit copied the file: {hit} bytes");
        let discarded = grew(&discard, &[], 1003);
        assert!(discarded >= 0, "invalidating gave back {discarded}");
    });
}

/// A native phar (no signature) holding `files`.
fn phar_bytes(files: &[(&str, &str)]) -> Vec<u8> {
    let le = |n: usize| (n as u32).to_le_bytes();
    let mut manifest: Vec<u8> = Vec::new();
    manifest.extend(le(files.len()));
    manifest.extend([0x11, 0x10]); // API version
    manifest.extend(le(0)); // global flags
    manifest.extend(le(0)); // alias
    manifest.extend(le(0)); // metadata
    let mut data: Vec<u8> = Vec::new();
    for (name, body) in files {
        manifest.extend(le(name.len()));
        manifest.extend(name.as_bytes());
        manifest.extend(le(body.len())); // size
        manifest.extend(le(500)); // mtime
        manifest.extend(le(body.len())); // compressed size
        manifest.extend(le(0)); // crc32
        manifest.extend(le(0o644)); // flags
        manifest.extend(le(0)); // metadata
        data.extend(body.as_bytes());
    }
    let mut out = b"<?php __HALT_COMPILER(); ?>\r\n".to_vec();
    out.extend(le(manifest.len()));
    out.extend(manifest);
    out.extend(data);
    out
}

#[test]
fn phar_scripts_are_cached_like_files() {
    // php's opcache caches phar:// scripts too (is_cacheable_stream_path):
    // PHPStan's workers include its whole phar with opcache on, and its
    // compiled code must not count against memory_limit.
    on_own_thread(|| {
        let d = Dir::new("phar");
        let body: String = (0..2000)
            .map(|i| format!("    $v{i} = 'value {i}' . PHP_EOL;\n"))
            .collect();
        let lib = format!("<?php\nfunction f() {{\n{body}}}\nreturn basename(__FILE__);\n");
        std::fs::write(d.0.join("lib.phar"), phar_bytes(&[("lib.php", &lib)])).unwrap();
        let main = d.file(
            "main.php",
            &format!(
                r#"<?php
                $before = memory_get_usage();
                $r = include 'phar://' . __DIR__ . '/lib.phar/lib.php';
                echo $r, ' ', memory_get_usage() - $before < 4096 ? 'small' : 'large', "\n";
                if (ini_get('opcache.enable')) {{ {STATS} }}"#
            ),
            500,
        );
        assert_eq!(
            request(&main, &[("opcache.enable", "0")], 1000),
            "lib.php large\n"
        );
        assert_eq!(
            request(&main, &[], 1001),
            "lib.php small\nhits=0 misses=2 scripts=2\n"
        );
        assert_eq!(
            request(&main, &[], 1002),
            "lib.php small\nhits=2 misses=2 scripts=2\n"
        );
    });
}

#[test]
fn another_name_is_served_by_its_key_without_a_stat() {
    on_own_thread(|| {
        let d = Dir::new("keys");
        std::fs::create_dir_all(d.0.join("sub")).unwrap();
        d.file("lib.php", "<?php return 'v1';", 500);
        let main = d.file(
            "main.php",
            &format!(
                "<?php echo include __DIR__ . '/sub/../lib.php', include 'lib.php', \"\\n\";{STATS}"
            ),
            500,
        );
        let ini = [("opcache.revalidate_freq", "2"), ("include_path", ".")];
        assert_eq!(
            request(&main, &ini, 1000),
            "v1v1\nhits=1 misses=2 scripts=2\n"
        );
        // Both names are keys now: found without resolving or opening the
        // file, and inside the revalidate window without a stat.
        d.file("lib.php", "<?php return 'v2';", 600);
        assert_eq!(
            request(&main, &ini, 1001),
            "v1v1\nhits=4 misses=2 scripts=2\n"
        );
        assert_eq!(
            request(&main, &ini, 1003),
            "v2v2\nhits=6 misses=3 scripts=2\n"
        );
    });
}

#[test]
fn the_realpath_cache_lives_as_long_as_the_thread() {
    on_own_thread(|| {
        let d = Dir::new("realpath");
        std::fs::create_dir_all(d.0.join("a")).unwrap();
        std::fs::create_dir_all(d.0.join("b")).unwrap();
        std::os::unix::fs::symlink(d.0.join("a"), d.0.join("cur")).unwrap();
        let main = d.file(
            "main.php",
            "<?php echo basename(realpath(__DIR__ . '/cur')), ' ', \
             count(array_filter(array_keys(realpath_cache_get()), \
             fn($k) => str_ends_with($k, '/cur'))), \"\\n\";",
            500,
        );
        assert_eq!(request(&main, &[], 1000), "a 1\n");
        // php-fpm's workers keep the cache between requests: the swapped
        // link still resolves to the old target until the TTL ends.
        std::fs::remove_file(d.0.join("cur")).unwrap();
        std::os::unix::fs::symlink(d.0.join("b"), d.0.join("cur")).unwrap();
        assert_eq!(request(&main, &[], 1001), "a 1\n");
        // Other settings: a cache of their own, empty.
        let off = [("realpath_cache_size", "0")];
        assert_eq!(request(&main, &off, 1002), "b 0\n");
        assert_eq!(request(&main, &[], 1003), "b 1\n");
    });
}
