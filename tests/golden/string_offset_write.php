<?php
// Writes through a string offset (zend_assign_to_string_offset) and the
// write-context errors for strings; `false` autovivifies with a deprecation.
set_error_handler(function ($no, $str, $file, $line) {
    echo "[$no] $str (line $line)\n";
    return true;
});
function t(callable $f) {
    try {
        $f();
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}

$s = "hi"; $s[5] = "x"; var_dump($s);
$s = "abc"; $s[-1] = "Z"; var_dump($s);
$s = "abc"; $s[-5] = "Z"; var_dump($s);
$s = "abc"; $s["1"] = "Z"; var_dump($s);
$s = "abc"; $s["1 "] = "W"; var_dump($s);
$s = "abc"; $s[1.5] = "QQ"; var_dump($s);
$s = "abc"; $s[1] = 7; var_dump($s);
$s = "abc"; $s[true] = "T"; var_dump($s);
$s = "abc"; $s[null] = "N"; var_dump($s);
$a = ['s' => 'abc']; $a['s'][1] = 'X'; var_dump($a);
$o = new stdClass; $o->s = 'abc'; $o->s[0] = 'Y'; var_dump($o->s);

t(function () { $x = ""; $x[] = 5; });
t(function () { $s = "hi"; $s[] = "x"; });
t(function () { $i = 5; $i[] = 1; });
t(function () { $s = "abc"; $s[0][0] = "x"; });
t(function () { $s = "abc"; $s[0] .= "x"; });
t(function () { $s = "abc"; $s[0]++; });
t(function () { $s = "abc"; $s["x"] = "Z"; });
t(function () { $s = "abc"; $s[1] = ""; });
t(function () { $s = "abc"; $r = &$s[0]; });
t(function () { $s = "abc"; $s[[]] = "W"; });

$f = false; $f[] = 1; var_dump($f);
$g = false; $g['k']['j'] = 2; var_dump($g);
$h = null; $h[] = 1; var_dump($h);
