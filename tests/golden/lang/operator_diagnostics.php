<?php
// Operator and conversion diagnostics (zend_operators.c slow paths, php 8.5).
set_error_handler(function ($no, $msg) {
    echo "  [$no] $msg\n";
});
function t(string $label, callable $f) {
    echo "$label:\n";
    try {
        var_dump($f());
    } catch (Throwable $e) {
        echo "  ", get_class($e), ": ", $e->getMessage(), "\n";
    }
}

// Arithmetic on strings: leading-numeric warns, non-numeric names both operands.
t('"5 apples" + "3"', fn() => "5 apples" + "3");
t('"a" - "b"', fn() => "a" - "b");
t('"1 " + 1', fn() => "1 " + 1);
t('[1] - [1]', fn() => [1] - [1]);
t('STDIN + 1', fn() => STDIN + 1);
t('-"x"', fn() => -"x");
t('+"7 up"', fn() => +"7 up");
t('+(-0.0)', fn() => +(-0.0));
t('null ** []', fn() => null ** []);

// % and shifts convert to int; strings throw, shift counts are checked.
t('"abc" % 3', fn() => "abc" % 3);
t('[1] % [1]', fn() => [1] % [1]);
t('7 % "2.5"', fn() => 7 % "2.5");
t('"8" << 64', fn() => "8" << 64);
t('"-8" >> 70', fn() => "-8" >> 70);
t('"1" << -1', fn() => "1" << -1);
t('1.5 | 4', fn() => 1.5 | 4);
t('1e30 & 1', fn() => 1e30 & 1);

// Bitwise on two strings is bytewise.
t('"ab" | "c"', fn() => bin2hex("ab" | "c"));
t('"ab" & "c"', fn() => bin2hex("ab" & "c"));
t('"ab" ^ "cde"', fn() => bin2hex("ab" ^ "cde"));
t('~"ab"', fn() => bin2hex(~"ab"));
t('~1.5', fn() => ~1.5);
t('~null', fn() => ~null);
t('~[]', fn() => ~[]);

// xor compares truthiness.
t('new stdClass xor 2', fn() => new stdClass xor 2);
t('"0" xor 0.0', fn() => "0" xor 0.0);

// Increment and decrement.
t('resource++', function () { $r = STDIN; $r++; return $r; });
t('[]--', function () { $a = []; $a--; return $a; });

// php 8.5 float to int and NAN.
$nan = fdiv(0, 0);
t('(int)1e19', fn() => (int)1e19);
t('(int)-INF', fn() => (int)-INF);
t('(int)"1e1000"', fn() => (int)"1e1000");
t('(int)"-1e30"', fn() => (int)"-1e30");
t('(bool)NAN', fn() => (bool)$nan);
t('(string)NAN', fn() => (string)$nan);
t('"x" . NAN', fn() => "x" . $nan);
t('if (NAN)', function () use ($nan) { if ($nan) { return "taken"; } return "not taken"; });
t('(array)NAN', fn() => (array)$nan);
t('[1.5 => "a"]', function () { $k = 1.5; return [$k => "a"]; });
t('[null => "a"]', function () { $k = null; return [$k => "a"]; });
t('[1e20 => "a"]', function () { $k = 1e20; return [$k => "a"]; });
t('0x1FFFFFFFFFFFFFFFF', fn() => 0x1FFFFFFFFFFFFFFFF);
t('100000000000000000000', fn() => 100000000000000000000);
function takes_int(int $i) { return $i; }
function takes_string(string $s) { return $s; }
t('takes_int(2.5)', fn() => takes_int(2.5));
t('takes_int("2.5")', fn() => takes_int("2.5"));
t('takes_string(NAN)', fn() => takes_string($nan));

// Offsets.
t('isset($a[[]])', function () { $a = [1]; return isset($a[[]]); });
t('empty($a[new stdClass])', function () { $a = [1]; return empty($a[new stdClass]); });
t('isset($a[null])', function () { $a = ["" => 1]; return isset($a[null]); });
t('isset($s["1x"])', function () { $s = "abc"; return isset($s["1x"]); });
t('$s["1x"] ?? "d"', function () { $s = "abc"; return $s["1x"] ?? "d"; });
t('$s[[]] ?? "d"', function () { $s = "abc"; return $s[[]] ?? "d"; });
t('unset($i[0][0])', function () { $i = 5; unset($i[0][0]); return $i; });
t('unset($o[0])', function () { $o = new stdClass; unset($o[0]); return $o; });
t('unset($f[0])', function () { $f = false; unset($f[0]); return $f; });
t('$a[$k] += 1 with $k = []', function () { $a = []; $k = []; $a[$k] += 1; return $a; });
t('$a[STDIN] = 1', function () { $a = []; $a[STDIN] = 1; return $a; });
t('array_key_exists(null, ...)', fn() => array_key_exists(null, ["" => 1]));
t('array_key_exists([], ...)', fn() => array_key_exists([], []));

// String offset assignment yields the byte written.
t('$x = $s[1] = "yz"', function () { $s = "abc"; $x = $s[1] = "yz"; return [$x, $s]; });
t('$x = $s[-9] = "q"', function () { $s = "abc"; $x = $s[-9] = "q"; return [$x, $s]; });
t('$s[0]->p = 1', function () { $s = "abc"; $s[0]->p = 1; });
t('unset($s[0]->p)', function () { $s = "abc"; unset($s[0]->p); });

// Builtins convert like zval_get_string / zval_get_long.
class Str { function __toString(): string { return "S"; } }
t('sprintf("%s-%d", new Str, "7x")', fn() => sprintf("%s-%d", new Str, "7x"));
t('sprintf("%d", 1e30)', fn() => sprintf("%d", 1e30));
t('sprintf("%s", [])', fn() => sprintf("%s", []));
t('sprintf("%s", new stdClass)', fn() => sprintf("%s", new stdClass));
t('vsprintf("%5.1f|%x", [new stdClass, 255])', fn() => vsprintf("%5.1f|%x", [new stdClass, 255]));
t('implode(",", [new Str, [], 1.5])', fn() => implode(",", [new Str, [], 1.5]));
t('strtr("ab", ["a" => new Str])', fn() => strtr("ab", ["a" => new Str]));
t('property_exists(1, "x")', fn() => property_exists(1, "x"));
t('method_exists(null, "x")', fn() => method_exists(null, "x"));
t('settype($n, "int")', function () use ($nan) { $n = $nan; settype($n, "int"); return $n; });
t('settype($n, "array")', function () use ($nan) { $n = $nan; settype($n, "array"); return $n; });
t('[] . "x" with an object', fn() => new stdClass . []);
