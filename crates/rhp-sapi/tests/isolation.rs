//! Requests run one after another on one thread must not see each other's
//! state: each [`run_request`] builds a fresh engine and resets the
//! builtins' thread-local state.

use rhp_sapi::{CaptureSapi, Captured, Outcome, Request, RequestInfo, Script};

// The engine's memory accounting (memory_get_usage(), memory_limit).
#[global_allocator]
static ALLOC: rhp_sapi::CountingAlloc = rhp_sapi::CountingAlloc;

fn run(code: &str, info: RequestInfo) -> (Captured, Outcome) {
    let req = Request {
        info: Some(info),
        ..Request::new(Script::Code(code.to_string()))
    };
    let (outcome, cap) = CaptureSapi::web("embed-test").run(req);
    (cap, outcome)
}

fn body(c: &Captured) -> String {
    String::from_utf8_lossy(&c.stdout).into_owned()
}

#[test]
fn sequential_requests_are_isolated() {
    let first = r#"
        ini_set('precision', '3');
        date_default_timezone_set('Asia/Tokyo');
        header('X-First: 1');
        http_response_code(404);
        setcookie('a', 'b');
        $GLOBALS['leak'] = 1;
        define('LEAKED', 1);
        function leaked_fn() {}
        class LeakedClass {}
        $f = fopen('php://memory', 'w+');
        mt_srand(1);
        echo $_GET['q'], ' ', php_sapi_name(), ' ', 1/3, "\n";
    "#;
    let info = RequestInfo {
        query_string: Some(b"q=one".to_vec()),
        method: Some(b"GET".to_vec()),
        response_code: 200,
        ..Default::default()
    };
    let (c1, o1) = run(first, info);
    assert_eq!(o1, Outcome::Exit(0));
    assert_eq!(body(&c1), "one embed-test 0.333\n");
    assert_eq!(c1.code, 404);
    assert!(c1.headers.iter().any(|h| h == b"X-First: 1"));

    let second = r#"
        var_dump(
            $_GET,
            ini_get('precision'),
            date_default_timezone_get(),
            isset($leak),
            defined('LEAKED'),
            function_exists('leaked_fn'),
            class_exists('LeakedClass'),
            http_response_code(),
            headers_list(),
        );
    "#;
    let info = RequestInfo {
        query_string: Some(b"r=two".to_vec()),
        method: Some(b"GET".to_vec()),
        response_code: 200,
        ..Default::default()
    };
    let (c2, o2) = run(second, info);
    assert_eq!(o2, Outcome::Exit(0));
    let powered = format!("X-Powered-By: PHP/{}", rhp_builtins::version::PHP_VERSION);
    let want = r#"array(1) {
  ["r"]=>
  string(3) "two"
}
string(2) "14"
string(3) "UTC"
bool(false)
bool(false)
bool(false)
bool(false)
int(200)
array(1) {
  [0]=>
  string(LEN) "POWERED"
}
"#
    .replace("LEN", &powered.len().to_string())
    .replace("POWERED", &powered);
    assert_eq!(body(&c2), want);
    assert_eq!(c2.code, 200);
    assert!(!c2.headers.iter().any(|h| h == b"X-First: 1"));
}

#[test]
fn sapi_name_follows_the_request() {
    let (c, _) = run(
        "echo PHP_SAPI, ' ', php_sapi_name();",
        RequestInfo::default(),
    );
    assert_eq!(body(&c), "embed-test embed-test");
    // A CLI-style request on the same thread afterwards (no SAPI request
    // input): the name is the new SAPI's again.
    let script = Script::Code("echo php_sapi_name();".to_string());
    let (_, out) = CaptureSapi::web("other").run(Request::new(script));
    assert_eq!(out.stdout, b"other");
}

// Builtin per-request state (ini overrides, default timezone, precision,
// strtok / preg / json state, putenv) must not leak from one CLI request to
// the next on the same thread, and putenv must not touch the process
// environment.

const DIRTY: &[u8] = br#"<?php
ini_set('precision', 5);
ini_set('serialize_precision', 3);
ini_set('arg_separator.output', ';');
ini_set('allow_url_fopen', '0');
error_reporting(0);
date_default_timezone_set('America/New_York');
strtok('a b c', ' ');
preg_replace('/(?:\D+|<\d+>)*[!?]/', '', 'foobar foobar foobar');
echo preg_last_error(), ' ';
json_decode('{');
putenv('RHP_REQUEST_STATE_TEST=dirty');
echo error_reporting(), ' dirty';
"#;

const CHECK: &[u8] = br#"<?php
echo 1/3, ' ', json_encode(0.1), ' ', http_build_query(['a' => 1, 'b' => 2]), ' ',
    ini_get('precision'), ' ', error_reporting(), ' ',
    date_default_timezone_get(), ' ', var_export(strtok(' '), true), ' ',
    preg_last_error(), ' ', json_last_error(), ' ',
    var_export(getenv('RHP_REQUEST_STATE_TEST'), true);
"#;

const CLEAN: &str = "0.33333333333333 0.1 a=1&b=2 14 30719 UTC false 0 0 false";

fn run_source(source: &[u8]) -> String {
    let script = Script::Source {
        path: "t.php".to_string(),
        bytes: source.to_vec(),
    };
    let (_, out) = CaptureSapi::cli().run(Request::new(script));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn vm_state_does_not_leak_between_engines() {
    let clean = run_source(CHECK);
    assert_eq!(clean, CLEAN);
    assert_eq!(run_source(DIRTY), "2 0 dirty");
    assert_eq!(run_source(CHECK), clean);
    assert!(std::env::var_os("RHP_REQUEST_STATE_TEST").is_none());
}

#[test]
fn clean_state_matches_php_defaults() {
    // Exactly what `php -n` prints (error_reporting = E_ALL = 30719; the
    // phpt harness passes its 32767 with -d, which rhp honours).
    assert_eq!(run_source(CHECK), CLEAN);
}

/// php drops gmp's random state at the end of a request (its RSHUTDOWN): a
/// seed does not carry over to the next engine.
#[test]
fn gmp_random_state_does_not_leak() {
    let seeded = run_source(
        b"<?php gmp_random_seed(42); echo gmp_random_bits(100), ' ', gmp_random_bits(100);",
    );
    let (first, second) = seeded.split_once(' ').expect("two numbers");
    assert_eq!(
        run_source(b"<?php gmp_random_seed(42); echo gmp_random_bits(100);"),
        first
    );
    assert_ne!(run_source(b"<?php echo gmp_random_bits(100);"), second);
}

#[test]
fn built_in_classes_are_fresh_in_every_request() {
    // The first request links the prelude's classes, the second keeps a
    // copy of them for the thread, the third starts from that copy.
    let code = r#"
        var_dump(class_exists('LeakedAlias'));
        class_alias('ArrayIterator', 'LeakedAlias');
        // Hoisted: a copy of the last request's class would redeclare it.
        class LeakedSub extends RuntimeException {}
        $e = new LeakedSub('m', 3);
        echo get_class($e), ' ', $e->getMessage(), ' ', $e->getCode(), ' ',
            get_parent_class($e), "\n";
        echo implode(',', class_implements(new LeakedAlias([1]))), "\n";
        echo count(get_declared_classes()), ' ', count(get_declared_interfaces()), "\n";
    "#;
    let outputs: Vec<String> = (0..3)
        .map(|_| {
            let (c, o) = run(code, RequestInfo::default());
            assert_eq!(o, Outcome::Exit(0), "{}", body(&c));
            body(&c)
        })
        .collect();
    assert!(
        outputs[0].starts_with("bool(false)\nLeakedSub m 3 RuntimeException\n"),
        "{}",
        outputs[0]
    );
    assert_eq!(outputs[1], outputs[0]);
    assert_eq!(outputs[2], outputs[0]);
}

/// A fresh directory below the system temp dir (canonical), removed on drop.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let base = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let p = base.join(format!("rhp-isolation-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("sub")).unwrap();
        Scratch(p)
    }

    fn path(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The permission bits std (0o666 under the process's umask) gives a new
/// file: the process's umask, read without changing it.
fn process_file_mode(dir: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    let f = dir.join("process-mode");
    std::fs::write(&f, b"").unwrap();
    let mode = std::fs::metadata(&f).unwrap().permissions().mode() & 0o777;
    std::fs::remove_file(&f).unwrap();
    mode
}

/// Requests on two threads at once, each in its own directory with its own
/// umask: chdir(), umask(), relative paths, file creation and child
/// processes stay per request, and the process's directory and umask never
/// change.
#[test]
fn threads_keep_their_own_cwd_and_umask() {
    let dirs = [Scratch::new("thread0"), Scratch::new("thread1")];
    // The first engine reads the process's umask (on macOS by setting it
    // for a moment): let that happen before measuring it.
    CaptureSapi::cli().run(Request::new(Script::Code(String::new())));
    let cwd_before = std::env::current_dir().unwrap();
    let mode_before = process_file_mode(&dirs[0].0);
    let code = r#"
        $dir = getcwd();
        $bad = [];
        $check = function ($what, $ok) use (&$bad, &$i) { if (!$ok) $bad[] = "$what@$i"; };
        for ($i = 0; $i < 150; $i++) {
            $check('start', getcwd() === $dir);
            $check('chdir', chdir('sub') && basename(getcwd()) === 'sub');
            umask(MASK);
            usleep(100);
            $check('umask', umask() === MASK);
            file_put_contents("f$i", 'x');
            clearstatcache();
            $check('file', is_file("$dir/sub/f$i") && (fileperms("$dir/sub/f$i") & 0777) === (0666 & ~MASK));
            mkdir("d$i");
            $check('dir', (fileperms("$dir/sub/d$i") & 0777) === (0777 & ~MASK));
            rmdir("d$i");
            unlink("f$i");
            if ($i % 50 === 0) $check('child', trim(shell_exec('pwd -P')) === "$dir/sub");
            $check('back', chdir('..') && getcwd() === $dir);
        }
        echo $bad ? implode(' ', $bad) : 'ok';
    "#;
    let workers: Vec<_> = dirs
        .iter()
        .enumerate()
        .map(|(t, d)| {
            let mask = ["0027", "0002"][t];
            let code = format!("<?php const MASK = {mask};\n{code}");
            let dir = d.path();
            std::thread::spawn(move || {
                let req = Request {
                    cwd: Some(dir.into_bytes()),
                    ..Request::new(Script::Source {
                        path: "t.php".to_string(),
                        bytes: code.into_bytes(),
                    })
                };
                let (o, out) = CaptureSapi::cli().run(req);
                (o, String::from_utf8_lossy(&out.stdout).into_owned())
            })
        })
        .collect();
    for w in workers {
        assert_eq!(
            w.join().expect("worker panicked"),
            (Outcome::Exit(0), "ok".to_string())
        );
    }
    assert_eq!(std::env::current_dir().unwrap(), cwd_before);
    assert_eq!(process_file_mode(&dirs[0].0), mode_before);
}

/// What one request changes in its file-system context and in the builtins'
/// per-request state is gone in the next request on the same thread.
#[test]
fn request_state_does_not_leak_on_one_thread() {
    let s = Scratch::new("leak");
    let dirty = format!(
        r#"<?php
        // A cycle: never freed, so its internal pointer is never forgotten.
        $p = new stdClass; $p->a = 1; $p->b = 2; $p->c = $p;
        @end($p); @prev($p);
        chdir('{dir}/sub');
        umask(0);
        [$a, $b] = stream_socket_pair(STREAM_PF_UNIX, STREAM_SOCK_STREAM, 0);
        @socket_get_option(socket_import_stream($a), SOL_SOCKET, 99999);
        @openssl_pkey_get_private('not a key');
        echo getcwd() === '{dir}/sub' ? 'in sub ' : 'not in sub ', umask(), ' ',
            socket_last_error() !== 0 ? 'socket error ' : 'no socket error ',
            openssl_error_string() !== false ? 'openssl error' : 'no openssl error';
        "#,
        dir = s.path()
    );
    let check = r#"<?php
        $p = new stdClass; $p->a = 1; $p->b = 2;
        echo getcwd(), ' ', umask(), ' ', socket_last_error(), ' ',
            var_export(openssl_error_string(), true), ' ', var_export(@current($p), true);
    "#;
    let run = |src: &str, cwd: Option<&str>| {
        let req = Request {
            cwd: cwd.map(|d| d.as_bytes().to_vec()),
            ..Request::new(Script::Source {
                path: "t.php".to_string(),
                bytes: src.as_bytes().to_vec(),
            })
        };
        let (_, out) = CaptureSapi::cli().run(req);
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let clean = run(check, None);
    assert_eq!(run(&dirty, None), "in sub 0 socket error openssl error");
    // Its engine is gone, and so is its state, before any next request.
    assert_eq!(rhp_builtins::zend::virtual_cwd::child_cwd(), None);
    assert_eq!(rhp_builtins::zend::virtual_cwd::child_umask(), None);
    assert_eq!(run(check, None), clean);
    // A request that starts elsewhere: only its own directory.
    let elsewhere = run(check, Some(&s.path()));
    assert!(
        elsewhere.starts_with(&format!("{} ", s.path())),
        "{elsewhere}"
    );
    assert_eq!(run(check, None), clean);
}

/// pcntl only under the CLI and CGI: signal handlers and fork belong to the
/// whole process, so a server that embeds rhp has none (rhp's choice; php's
/// fpm and embed builds can have pcntl).
#[test]
fn pcntl_is_only_in_cli_and_cgi() {
    let code = "echo var_export(function_exists('pcntl_signal'), true), ' ', \
                var_export(extension_loaded('pcntl'), true), ' ', \
                var_export(defined('SIGTERM'), true), ' ', \
                var_export(in_array('pcntl', get_loaded_extensions()), true);";
    let (c, _) = run(code, RequestInfo::default());
    assert_eq!(body(&c), "false false false false");
    let script = Script::Code(code.to_string());
    let (_, out) = CaptureSapi::cli().run(Request::new(script));
    assert_eq!(out.stdout, b"true true true true");
    let (_, out) = CaptureSapi::web("cgi-fcgi").run(Request::new(Script::Code(
        "echo var_export(function_exists('pcntl_fork'), true);".to_string(),
    )));
    assert!(out.stdout.ends_with(b"true"), "{:?}", out.stdout);
}
