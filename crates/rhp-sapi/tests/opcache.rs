//! The compiled-script cache across requests on one thread: a script is
//! reused while its file is unchanged, revalidated as php's settings say,
//! never shares per-request state, and preloaded declarations are fresh in
//! every request.
//!
//! The cache is per thread, so each test runs on a thread of its own.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, UNIX_EPOCH};

use rhp_sapi::{run_request, Outcome, Request, RequestInfo, Sapi, Script};

#[derive(Clone, Default)]
struct Capture {
    body: Rc<RefCell<Vec<u8>>>,
}

impl Sapi for Capture {
    fn name(&self) -> &str {
        "embed-test"
    }

    fn ub_write(&mut self, data: &[u8]) {
        self.body.borrow_mut().extend_from_slice(data);
    }
}

/// One request for `script` at `time` (seconds since the epoch), with
/// opcache on and `ini` on top; returns the body.
fn request(script: &Path, ini: &[(&str, &str)], time: u64) -> String {
    let cap = Capture::default();
    let mut settings = vec![
        ("opcache.enable".to_string(), "1".to_string()),
        ("display_errors".to_string(), "1".to_string()),
    ];
    settings.extend(ini.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    let outcome = run_request(
        Box::new(cap.clone()),
        Request {
            script: Script::File(script.to_string_lossy().into_owned()),
            ini: settings,
            args: Vec::new(),
            info: Some(RequestInfo {
                method: Some(b"GET".to_vec()),
                response_code: 200,
                request_time: time as f64,
                ..Default::default()
            }),
            php_binary: None,
        },
    );
    assert_eq!(outcome, Outcome::Exit(0));
    let body = String::from_utf8_lossy(&cap.body.borrow()).into_owned();
    body
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
