<?php
// #[\NoDiscard], (void), #[\Deprecated] and #[\SensitiveParameter] at run
// time, in the places the phpt files leave out.
set_error_handler(function ($no, $str, $file, $line) {
    echo "[$no] $str (line $line)\n";
    return true;
});

#[\NoDiscard("keep it")]
function nd(int $x = 1): int { return $x; }

// A builtin's callback, a used result, a nested call, a parenthesized one.
array_map('nd', [1, 2]);
$r = nd();
echo nd(nd()), "\n";
(nd());
(void)nd();
for ($i = 0, nd(); $i < 1; $i++, (void)nd()) {}
call_user_func('nd');
$f = 'call_user_func';
$f('nd');
$c = nd(...);
$c();
(void)$c();

$d = new DateTimeImmutable('2000-01-01 00:00:00', new DateTimeZone('UTC'));
$d->modify('+1 day');
echo $d->modify('+1 day')->format('Y-m-d'), "\n";

#[\Deprecated(since: "1.2")]
function dep() { return 1; }
dep();
array_map('dep', [1]);
echo (new ReflectionFunction('dep'))->isDeprecated() ? "deprecated\n" : "not\n";

#[\Deprecated("use OTHER")]
const OLD = 1;
echo constant('OLD'), "\n";
echo defined('OLD') ? "defined\n" : "undefined\n";
var_dump((new ReflectionConstant('OLD'))->isDeprecated());
echo constant('FILE_TEXT'), "\n";

class K {
    #[\Deprecated]
    const A = 1;
    #[\Deprecated("named")]
    public function m(#[\SensitiveParameter] $secret, ...$rest) {
        throw new Exception('boom');
    }
}
echo constant('K::A'), "\n";
try {
    (new K)->m('pw', 'x');
} catch (Exception $e) {
    foreach ($e->getTrace()[0]['args'] as $a) {
        echo is_object($a) ? get_class($a) : $a, "\n";
    }
}

try {
    $v = new SensitiveParameterValue('hidden');
    var_dump(serialize($v) === 'O:23:"SensitiveParameterValue":0:{}', get_object_vars($v));
    echo $v->getValue(), "\n";
} catch (Exception $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}

// A destructor of a released temporary throws inside the try.
class D { function __destruct() { throw new Exception('dtor'); } }
try { new D(); } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
