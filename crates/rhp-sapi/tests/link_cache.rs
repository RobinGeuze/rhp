//! The linked-class cache (crates/rhp-vm/src/link_cache.rs): from a
//! thread's second request on, a user class linked from the same inputs
//! as before is installed from the cache. Every request must print what
//! a fresh link prints, whatever the requests before it did.
//!
//! Each script runs once on a thread of its own for its reference output
//! (a thread's first request links everything), then the scripts run
//! interleaved on one thread.

use std::path::{Path, PathBuf};

use rhp_sapi::{CaptureSapi, Outcome, Request, RequestInfo, Script};

#[global_allocator]
static ALLOC: rhp_sapi::CountingAlloc = rhp_sapi::CountingAlloc;

fn run(script: &Path) -> String {
    let req = Request {
        ini: vec![
            ("opcache.enable".to_string(), "1".to_string()),
            (
                "opcache.file_update_protection".to_string(),
                "0".to_string(),
            ),
            ("opcache.revalidate_freq".to_string(), "0".to_string()),
            ("display_errors".to_string(), "1".to_string()),
            ("error_reporting".to_string(), "-1".to_string()),
        ],
        info: Some(RequestInfo {
            method: Some(b"GET".to_vec()),
            response_code: 200,
            ..Default::default()
        }),
        ..Request::new(Script::File(script.to_string_lossy().into_owned()))
    };
    let (outcome, cap) = CaptureSapi::web("embed-test").run(req);
    let out = String::from_utf8_lossy(&cap.stdout).into_owned();
    assert!(matches!(outcome, Outcome::Exit(_)), "{outcome:?}: {out}");
    out
}

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("rhp-linkcache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("lib")).unwrap();
        Dir(d.canonicalize().unwrap())
    }

    fn file(&self, name: &str, src: &str) -> PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, src).unwrap();
        p
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const AUTOLOAD: &str = r#"<?php
spl_autoload_register(function ($c) {
    echo "load $c\n";
    require __DIR__ . "/lib/$c.php";
});
"#;

const LIB: &[(&str, &str)] = &[
    ("I1", "interface I1 extends Countable { const IC = 'ic'; function size(): int; }"),
    (
        "T1",
        "trait T1 { public static $made = 0; private array $bag = []; \
         public function count(): int { return count($this->bag); } \
         public function put($k, $v): static { $this->bag[$k] = $v; self::$made++; return $this; } }",
    ),
    ("Bar", "class Bar {}"),
    ("Other", "class Other { const X = 'x'; }"),
    ("Foo", "class Foo extends Bar {}"),
    (
        "Base",
        "abstract class Base implements I1 { use T1; \
         const A = self::B . '-a'; const B = 'b'; const O = Other::X . '!'; \
         public static $s = [1]; protected static int $n = 0; public $p = self::A; \
         public function size(): int { return static::$n; } \
         public function make(): Bar { return new Bar; } }",
    ),
    (
        "Child",
        "class Child extends Base { const C = parent::A . '-c'; public static $own = 'own'; \
         public function make(): Bar { return new Foo; } }",
    ),
    (
        "Ret",
        "class Ret extends Base { public function make(): Foo { return new Foo; } }",
    ),
    (
        "Ser",
        "class Ser implements Serializable { \
         public function serialize() { return ''; } public function unserialize($s) {} }",
    ),
    (
        "Status",
        "enum Status: string implements I1 { case A = 'a'; case B = 'b'; const DEF = self::B; \
         public function count(): int { return 2; } public function size(): int { return 1; } }",
    ),
    (
        "WithEnum",
        "class WithEnum { const S = Status::A; public Status $st = Status::B; \
         public static $list = [Status::A]; }",
    ),
];

/// Uses the classes in one order: parent constants evaluated before the
/// children link, Foo loaded before Ret's variance check needs it.
const A: &str = r#"
echo Base::A, Base::O, "\n";
new Foo;
$c = new Child;
Child::$s[] = 2;
echo count(Base::$s), " ", Child::C, " ", $c->p, "\n";
$r = new Ret;
echo get_class($r->make()), " ", Ret::$made, "\n";
echo Status::DEF->value, " ", Status::A->name, "\n";
var_dump(WithEnum::S === Status::A, WithEnum::$list[0] === Status::A, (new WithEnum)->st);
$o = new Ser;
print_r(class_implements('Child'));
print_r(get_class_methods('Ret'));
print_r(array_keys(get_class_vars('Child')));
"#;

/// The other order: a child links before its parent's constants are
/// evaluated (Child::O then loads Other), Ret before Foo is loaded (its
/// variance check autoloads Foo), statics changed through the child.
const B: &str = r#"
echo Child::C, "\n";
echo "before O\n", Child::O, "\n";
$r = new Ret;
$c = new Child;
$c->put('x', 1)->put('y', 2);
echo count($c), " ", Child::$made, " ", Base::$made, "\n";
Child::$s = ['z'];
echo implode(',', Base::$s), " ", Child::C, " ", Base::A, "\n";
var_dump(Status::from('b'), WithEnum::S);
echo Status::DEF->value, "\n";
$o = new Ser;
print_r(class_parents($r));
"#;

#[test]
fn cached_links_print_what_fresh_links_print() {
    let d = Dir::new("orders");
    for (name, src) in LIB {
        d.file(&format!("lib/{name}.php"), &format!("<?php\n{src}\n"));
    }
    let a = d.file("a.php", &format!("{AUTOLOAD}{A}"));
    let b = d.file("b.php", &format!("{AUTOLOAD}{B}"));
    let fresh = |p: &PathBuf| {
        let p = p.clone();
        std::thread::spawn(move || run(&p)).join().unwrap()
    };
    let (ra, rb) = (fresh(&a), fresh(&b));
    assert!(rb.contains("load Foo"), "{rb}");
    assert!(rb.contains("before O\nload Other\n"), "{rb}");
    assert!(
        ra.contains("Serializable interface, which is deprecated"),
        "{ra}"
    );
    let (a2, b2) = (a.clone(), b.clone());
    let hits = std::thread::spawn(move || {
        for p in [&a2, &b2, &a2, &a2, &b2, &b2, &a2, &b2] {
            let want = if p == &a2 { &ra } else { &rb };
            assert_eq!(&run(p), want, "{}", p.display());
        }
        rhp_vm::link_cache_stats()
    })
    .join()
    .unwrap();
    // Most classes come from the cache: each order links alike every time.
    assert!(hits.0 > 20, "{hits:?}");
}

#[test]
fn a_changed_dependency_links_again() {
    let d = Dir::new("changed");
    let a = d.file(
        "lib/P.php",
        "<?php class P { function f(): int { return 1; } }\n",
    );
    let main = d.file(
        "main.php",
        &format!("{AUTOLOAD}echo (new C)->f(), ' ', get_parent_class(new C), \"\\n\";\n"),
    );
    d.file("lib/C.php", "<?php class C extends P {}\n");
    let main2 = main.clone();
    std::thread::spawn(move || {
        assert_eq!(run(&main2), "load C\nload P\n1 P\n");
        assert_eq!(run(&main2), "load C\nload P\n1 P\n");
        assert_eq!(run(&main2), "load C\nload P\n1 P\n");
        // A new parent (a new compile of P): C links against it.
        std::fs::write(&a, "<?php class P { function f(): int { return 22; } }\n").unwrap();
        let f = std::fs::File::options().write(true).open(&a).unwrap();
        f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
            .unwrap();
        let out = run(&main2);
        assert_eq!(out, "load C\nload P\n22 P\n");
    })
    .join()
    .unwrap();
}

#[test]
fn a_parent_constant_evaluated_first_is_an_input() {
    // The same files in the same order; one script evaluates P's constant
    // (and so its deprecation) before K links, the other through K.
    let d = Dir::new("state");
    d.file(
        "lib/P.php",
        "<?php class P { #[\\Deprecated] const DEP = 1; const O = self::DEP + 1; }\n",
    );
    d.file("lib/K.php", "<?php class K extends P {}\n");
    let first = d.file(
        "first.php",
        "<?php require __DIR__ . '/lib/P.php'; echo P::O, \"\\n\"; \
         require __DIR__ . '/lib/K.php'; echo K::O, \"\\n\";\n",
    );
    let through = d.file(
        "through.php",
        "<?php require __DIR__ . '/lib/P.php'; require __DIR__ . '/lib/K.php'; \
         echo K::O, \"\\n\", P::O, \"\\n\";\n",
    );
    let fresh = |p: &PathBuf| {
        let p = p.clone();
        std::thread::spawn(move || run(&p)).join().unwrap()
    };
    let (rf, rt) = (fresh(&first), fresh(&through));
    // Through K, the deprecation comes before the first line.
    assert!(rt.find("is deprecated") < rt.find("2\n"), "{rt}");
    std::thread::spawn(move || {
        // Three of the first kind, so its link is cached (the second
        // time it links, from the thread's second request) before the
        // other kind runs.
        for p in [&first, &first, &first, &through, &first, &through] {
            let want = if p == &first { &rf } else { &rt };
            assert_eq!(&run(p), want);
        }
        let (hits, _) = rhp_vm::link_cache_stats();
        // P and K in the last two requests, P in the first `through`.
        assert_eq!(hits, 5);
    })
    .join()
    .unwrap();
}

#[test]
fn a_class_a_variance_check_looked_up_is_an_input() {
    // Ret's check of `make(): Foo` against `make(): Bar` looks Foo up; the
    // two scripts declare different Foos at the same place.
    let d = Dir::new("lookup");
    d.file("lib/Bar.php", "<?php class Bar {}\n");
    d.file(
        "lib/Base.php",
        "<?php class Base { function make(): Bar { return new Bar; } }\n",
    );
    d.file(
        "lib/Ret.php",
        "<?php class Ret extends Base { function make(): Foo { return new Foo; } }\n",
    );
    let main = |foo: &str| {
        format!(
            "<?php require __DIR__ . '/lib/Bar.php'; require __DIR__ . '/lib/Base.php';\n\
             {foo}\nrequire __DIR__ . '/lib/Ret.php'; echo get_class((new Ret)->make()), \"\\n\";\n"
        )
    };
    // (Declared conditionally, so neither is hoisted before Bar.)
    let sub = d.file("sub.php", &main("if (true) { class Foo extends Bar {} }"));
    let other = d.file("other.php", &main("if (true) { class Foo {} }"));
    std::thread::spawn(move || {
        for _ in 0..3 {
            assert_eq!(run(&sub), "Foo\n");
        }
        let out = run(&other);
        assert!(
            out.contains("Fatal error: Declaration of Ret::make(): Foo must be compatible"),
            "{out}"
        );
    })
    .join()
    .unwrap();
}
