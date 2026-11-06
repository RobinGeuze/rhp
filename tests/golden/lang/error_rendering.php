<?php
// Error and trace rendering (php's zend_exceptions.c / zend_builtin_functions.c).
function clean($s) { return str_replace([__FILE__, __DIR__], ['F', 'D'], $s); }

echo "-- @ restores error_reporting set to fatal-only inside\n";
error_reporting(E_ALL);
function quiet() { error_reporting(0); }
@quiet();
var_dump(error_reporting());
function narrower() { error_reporting(E_ALL & ~E_NOTICE); }
@narrower();
var_dump(error_reporting());
error_reporting(E_ALL);

echo "-- anonymous class names\n";
$o = new class {
    public int $i = 0;
    public self $me;
    public function m(int $x) {}
    private function p() {}
};
try { $o->i = "x"; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { $o->me = 1; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { $o->m("x"); } catch (TypeError $e) { echo clean($e->getMessage()), "\n"; }
try { $o->p(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $o->nope(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump(new class { private $a = 1; protected $b = 2; public $c = 3; });
print_r(new class { private $a = 1; });
echo "\n";
$e1 = eval('return new class {};');
$e2 = eval('return new class {};');
var_dump(get_class($e1) === get_class($e2));

echo "-- trace arguments\n";
function t($s, ...$rest) { return clean((new Exception)->getTraceAsString()); }
echo t('abcdefghijklmnopqrstuvwxyz', 1.0, [1], null, true), "\n";
var_dump(ini_set('zend.exception_string_param_max_len', '3'));
echo t('abcdef'), "\n";
var_dump(ini_set('zend.exception_string_param_max_len', '-1'));
var_dump(ini_get('zend.exception_string_param_max_len'));
ini_set('zend.exception_ignore_args', '1');
echo t('abcdef'), "\n";
try { echo match('x') {}; } catch (\UnhandledMatchError $e) { echo $e->getMessage(), "\n"; }
ini_set('zend.exception_ignore_args', '0');
ini_set('zend.exception_string_param_max_len', '15');

echo "-- constant expressions\n";
class K {
    const A = [
        UNDEF_X,
    ];
    const B = UNDEF_Y;
}
try { K::A; } catch (Error $e) { echo $e->getMessage(), ' @', $e->getLine(), "\n", clean($e->getTraceAsString()), "\n"; }
try { K::B; } catch (Error $e) { echo $e->getMessage(), ' @', $e->getLine(), "\n", clean($e->getTraceAsString()), "\n"; }

echo "-- throw and class names\n";
foreach ([1, null, 'str', new stdClass, function () {}] as $v) {
    try { throw $v; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
}
foreach ([1, null, [], 1.5] as $x) {
    try { $x::m(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    try { new $x; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}

echo "-- static return type\n";
class SA { public function make(): static { return new SA; } }
class SB extends SA {}
try { var_dump((new SB)->make()); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$c = function (): static { return new SA; };
try { $c(); } catch (TypeError $e) { echo clean($e->getMessage()), "\n"; }
var_dump(Closure::bind($c, new SA, SA::class)());

echo "-- __toString return type\n";
class TS { public function __toString() { return 42; } }
var_dump((string) new TS);
class TN { public function __toString() {} }
try { echo new TN; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }

echo "-- exception string cache\n";
$e = new LogicException('m');
$s = (string) $e;
$p = new ReflectionProperty(Exception::class, 'string');
var_dump($p->getValue($e) === $s);
$p = new ReflectionProperty(Exception::class, 'trace');
$p->setValue($e, [['file' => 'x', 'line' => 'y', 'function' => 'f', 'args' => [1, 'two']], 'bad']);
set_error_handler(function ($no, $msg) { echo "warning: $msg\n"; });
echo $e->getTraceAsString(), "\n";
