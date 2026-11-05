<?php
// Parameters whose bodies used to ignore them.
set_error_handler(function ($no, $msg, $file, $line) {
    echo "[$no] $msg @", basename($file), ":$line\n";
    return true;
});
class S { function __toString(): string { return 'sa'; } }
var_dump(str_replace('a', 'b', ['aa', ['a'], new S, 5], $c), $c);
var_dump(str_ireplace(['A', ['x']], 'z', 'aArray', $c), $c);
var_dump(strip_tags('<a>x</a><b>y</b>', [new S]));
var_dump(count([1, [2, [3, 4]]], COUNT_RECURSIVE), sizeof([1, [2]], 1));
$r = [1];
$r[] = &$r;
var_dump(count($r, COUNT_RECURSIVE));
try { count([], 2); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { sizeof(1); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
var_dump(htmlentities("<a>&amp;'\"\u{e9}", ENT_NOQUOTES, 'UTF-8', false));
var_dump(htmlentities("'\u{e9}", ENT_QUOTES | ENT_XHTML), htmlentities("\xe9", ENT_QUOTES, 'ISO-8859-1'));
var_dump(html_entity_decode('&quot;&#039;&eacute;&apos;', ENT_NOQUOTES), html_entity_decode('&apos;&eacute;', ENT_QUOTES | ENT_XML1));
var_dump(htmlspecialchars_decode('&quot;&#039;&#x27;&apos;', ENT_COMPAT), htmlspecialchars_decode('&apos;', ENT_QUOTES | ENT_HTML5));
var_dump(json_encode([[1]], 0, 1), json_last_error_msg(), json_encode([], 0, 0), json_encode([[1], 2], JSON_PARTIAL_OUTPUT_ON_ERROR, 1));
foreach ([PHP_ROUND_HALF_UP, PHP_ROUND_HALF_DOWN, PHP_ROUND_HALF_EVEN, PHP_ROUND_HALF_ODD, 5, 6, 7, 8] as $m) {
    echo $m, ': ', round(2.5, 0, $m), ' ', round(-2.5, 0, $m), ' ', round(1.955, 2, $m), ' ', round(1234.5, -2, $m), "\n";
}
try { round(1, 0, 9); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
define('CI_CONST', 1, true);
var_dump(clearstatcache(), (string) new GMP('ff', 16), function_exists('sapi_windows_vt100_support'));
$o = new stdClass;
$o->p = 10;
$o->q = 11;
var_dump(current($o), key($o), next($o), key($o), next($o), reset($o), end($o));
