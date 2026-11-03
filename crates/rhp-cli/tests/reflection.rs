//! Differential tests for Reflection, dynamic member access, by-ref foreach over
//! lvalues, and reference binding to lvalues (the Symfony/Composer-driven
//! features). Compares the built `rhp` against the real `php`; skips without php.

use std::path::{Path, PathBuf};
use std::process::Command;

fn php_available() -> bool {
    Command::new("php")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn case(name: &str, src: &str) {
    if !php_available() {
        eprintln!("[reflection] no php — skipping");
        return;
    }
    let dir: PathBuf = std::env::temp_dir().join(format!("rhp_refl_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("t.php");
    std::fs::write(&f, src).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&run("php", &f)),
        String::from_utf8_lossy(&run(env!("CARGO_BIN_EXE_rhp"), &f)),
        "output differs from php for case {name}"
    );
}

fn run(bin: &str, file: &Path) -> Vec<u8> {
    let args: &[&str] = if bin == "php" { &["-n"] } else { &[] };
    Command::new(bin)
        .args(args)
        .arg(file)
        .output()
        .expect("run")
        .stdout
}

#[test]
fn reflection_class_method_property() {
    case(
        "refl",
        r#"<?php
        namespace App;
        interface Greets {}
        abstract class Base { const VERSION = '1.0'; public $x = 1; }
        class Thing extends Base implements Greets {
            private $secret = 42;
            public function __construct(public int $id = 7) {}
            public function hi(string $n): string { return "hi $n #$this->id"; }
        }
        $r = new \ReflectionClass(Thing::class);
        echo $r->getName(), " / ", $r->getShortName(), " / ", $r->getNamespaceName(), "\n";
        var_dump($r->isAbstract(), $r->isInstantiable());
        echo $r->getParentClass()->getName(), "\n";
        var_dump($r->implementsInterface(Greets::class), $r->isSubclassOf(Base::class));
        $o = $r->newInstance(99);
        $m = new \ReflectionMethod(Thing::class, 'hi');
        echo $m->invoke($o, 'bob'), "\n";
        foreach ($m->getParameters() as $p) {
            echo $p->getName(), ":", ($p->getType() ? $p->getType()->getName() : '?'), " ";
        }
        echo "\n";
        $prop = new \ReflectionProperty($o, 'secret');
        echo $prop->getValue($o), "\n";
        $prop->setValue($o, 100);
        echo $prop->getValue($o), "\n";
        echo $r->getConstant('VERSION'), "\n";
        "#,
    );
}

#[test]
fn dynamic_member_access() {
    case(
        "dyn",
        r#"<?php
        class Box {
            public $a = 1;
            function greet($n) { return "hi $n"; }
        }
        $o = new Box();
        $p = 'a'; echo $o->$p, "\n";
        $o->$p = 10; echo $o->a, "\n";
        $o->{'a'} = 20; echo $o->{'a'}, "\n";
        $m = 'greet'; echo $o->$m('x'), "\n";
        "#,
    );
}

#[test]
fn by_ref_foreach_and_reference_binding() {
    case(
        "refs",
        r#"<?php
        class C { public $items = [1, 2, 3]; function dbl() { foreach ($this->items as &$v) $v *= 2; } }
        $c = new C(); $c->dbl(); echo implode(',', $c->items), "\n";
        $a = []; $x = 5; $a['k'] =& $x; $x = 9; echo $a['k'], "\n";
        $o = new C(); $v = 1; $o->items['n'] =& $v; $v = 7; echo $o->items['n'], "\n";
        $ref =& $c->items[0]; $ref = 100; echo $c->items[0], "\n";
        "#,
    );
}

/// Only prelude code can call the native `__reflect*` helpers, but a
/// Reflection subclass can forge the protected state the prelude passes
/// them: garbage there must raise or return, never panic the engine.
#[test]
fn reflect_helpers_survive_garbage() {
    let dir: PathBuf = std::env::temp_dir().join("rhp_refl_garbage");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("t.php");
    std::fs::write(
        &f,
        r#"<?php
        error_reporting(0);
        trait Forge { function forge($slot, $v) { $this->$slot = $v; return $this; } }
        class F extends ReflectionFunction { use Forge; }
        class M extends ReflectionMethod { use Forge; }
        class C extends ReflectionClass { use Forge; }
        class O extends ReflectionObject { use Forge; }
        $vals = [null, 0, -1, PHP_INT_MAX, '', "\0", 'stdClass', [], [1], ['stdClass'],
                 [1, 2, 3], new stdClass, function () {}, [new stdClass, 'x']];
        $subjects = [
            [F::class, ['strlen'], ['__i', '__l']],
            [M::class, ['ArrayObject', 'count'], ['__i', '__l']],
            [C::class, ['ArrayObject'], ['__c', '__o']],
            [O::class, [new ArrayObject([1])], ['__c', '__o']],
        ];
        foreach ($subjects as [$cls, $ctor, $slots]) {
            foreach ($slots as $slot) {
                foreach ($vals as $v) {
                    $o = (new $cls(...$ctor))->forge($slot, $v);
                    foreach (get_class_methods($o) as $m) {
                        if (in_array($m, ['forge', '__construct'], true)) { continue; }
                        foreach ([[], [$v], [$v, $v]] as $args) {
                            try { $o->$m(...$args); } catch (Throwable $e) {}
                        }
                    }
                }
            }
        }
        echo "ok\n";
        "#,
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rhp"))
        .arg(&f)
        .output()
        .expect("run rhp");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success() && out.stdout.ends_with(b"ok\n"),
        "rhp failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
