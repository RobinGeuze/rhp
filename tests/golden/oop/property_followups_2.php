<?php
// Review follow-ups: the __clone window closes when __clone returns,
// set visibility of statics on references and unset, typed statics in
// Reflection, and isset on a non-object with a dynamic name.

class C {
    public bool $again = false;
    public function __construct(public readonly int $x) {}
    public function __clone() { if ($this->again) { $this->x = 99; } }
}
$b = clone new C(1);
$b->again = true;
try { $b->__clone(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump($b->x);

class T {
    public static private(set) array $a = [1, 2];
    public static private(set) ?T $o = null;
    public $v;
    public static function init() { self::$o = new T; }
}
T::init();
T::$o->v = 3; // an object below a nested write is fine
var_dump(T::$o->v);
// (php leaves the slot a reference after this error: nothing below
// writes through it.)
try { $r = &T::$o; } catch (Error $e) { echo $e->getMessage(), "\n"; }
function f(&$x) {}
try { f(T::$o); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { unset(T::$a[0]); } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump(T::$a);

class U { public static int $u; public static int $d = 3; }
$rc = new ReflectionClass('U');
try { $rc->getStaticPropertyValue('u'); } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump($rc->getStaticPropertyValue('u', 'def'));
try { $rc->getStaticPropertyValue('nope'); } catch (ReflectionException $e) { echo $e->getMessage(), "\n"; }
$rp = new ReflectionProperty('U', 'u');
var_dump($rp->isInitialized(), (new ReflectionProperty('U', 'd'))->isInitialized());
try { $rp->getValue(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
U::$u = 4;
var_dump($rp->isInitialized(), $rp->getValue());

class S { public function __toString(): string { echo "toString\n"; return "x"; } }
$s = new S;
var_dump(isset($null->$s), empty($null->$s), $null->$s ?? 2);
var_dump(isset($null->{$undefined}));
