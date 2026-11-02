<?php
// Writes through string offsets and the diagnostics of nested / compound
// writes (php's zend_assign_to_string_offset, zend_wrong_string_offset_error,
// the read-for-write "Undefined array key" warnings, false autovivification).
set_error_handler(function ($no, $msg, $file, $line) {
    echo "[$no] $msg (line $line)\n";
    return true;
});
function attempt(callable $f) {
    try {
        $f();
    } catch (Throwable $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}

$s = 'abc';
$s[1] = 'x';
$s[5] = 'y';
$s[-1] = 'z';
$s['0'] = 'Q';
$s[1] = 'long';
var_dump($s);
$s[-10] = 'n';
$s[1.0] = 'f';
$s['2x'] = 'i';
var_dump($s);
$e = '';
$e[2] = 'a';
var_dump($e);
$nested = ['s' => 'abc'];
$nested['s'][0] = 'X';
var_dump($nested);

attempt(function () { $s = 'abc'; $s[1] = ''; });
attempt(function () { $s = 'abc'; $s[] = 'd'; });
attempt(function () { $s = ''; $s[] = 'd'; });
attempt(function () { $s = 'abc'; $s['x'] = 'd'; });
attempt(function () { $s = 'abc'; $s[0][0] = 'd'; });
attempt(function () { $s = 'abc'; $s['x']['y'] = 'd'; });
attempt(function () { $s = 'abc'; $s[0] .= 'd'; });
attempt(function () { $s = 'abc'; $s[0]++; });
attempt(function () { $s = 'abc'; $r = &$s[0]; });
attempt(function () { $s = 'abc'; unset($s[0]); });
attempt(function () { $s = 'abc'; unset($s[0][0]); });
attempt(function () { $i = 5; unset($i[0]); });
attempt(function () { $a = []; $a[new stdClass] = 1; });
attempt(function () { $a = []; $a[[]]['y'] = 1; });
attempt(function () { $a = [1]; unset($a[[]]); });

$f = false;
$f[] = 1;
var_dump($f);
$g = ['f' => false];
$g['f']['k'] = 2;
var_dump($g);
$h = false;
unset($h[0]);

$m = [];
$m['z'] .= 'q';
$m['a']['b'] += 2;
$m['n']++;
var_dump($m);
$o = new stdClass;
$o->arr['k'] .= 'x';
print_r($o);

$k = [];
$k[null] = 'null key';
$k[1.5] = 'float key';
var_dump($k);
