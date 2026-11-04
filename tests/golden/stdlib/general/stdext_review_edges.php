<?php
// Edge cases of the tooling builtins against php: settype on objects,
// array_multisort argument naming, HashContext's hidden state and
// hash_update_file's warning, mb_detect_encoding / mb_check_encoding,
// preg_match(_all) start offsets, array_fill's size limit.
set_error_handler(function ($no, $msg) { echo "[$no] $msg\n"; return true; });

class S { function __toString() { return "str"; } }
foreach (['int', 'float', 'string', 'bool'] as $t) {
    foreach ([new stdClass, new S, fopen('php://memory', 'r'), fn() => 1] as $v) {
        try { settype($v, $t); var_dump($v); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
    }
}
$a = [1]; try { array_multisort($a, SORT_ASC, SORT_DESC); } catch (Throwable $e) { echo $e->getMessage(), "\n"; }
try { array_multisort(SORT_ASC); } catch (Throwable $e) { echo $e->getMessage(), "\n"; }
$a = [1]; try { array_multisort($a, "x"); } catch (Throwable $e) { echo $e->getMessage(), "\n"; }
$a = [1]; try { array_multisort($a, 99); } catch (Throwable $e) { echo $e->getMessage(), "\n"; }

$hc = hash_init('md5');
var_dump((array) $hc, get_object_vars($hc), json_encode($hc));
var_dump(hash_update_file($hc, '/nonexistent/rhp-review-file'));
var_dump(@hash_update_file($hc, '/nonexistent/rhp-review-file'));

$lists = [null, 'ASCII', 'ISO-8859-1', 'ASCII,UTF-8', 'UTF-8,ASCII', ['ISO-8859-1', 'UTF-8'], ['UTF-8', 'ISO-8859-1'], 'auto'];
foreach (["abc", "", "\xe9", "caf\xc3\xa9", "\xc3\xa9\xe9"] as $str) {
    foreach ($lists as $l) {
        echo bin2hex($str), ' ', json_encode($l), ' ', var_export(mb_detect_encoding($str, $l), true), ' ', var_export(mb_detect_encoding($str, $l, true), true), "\n";
    }
}
try { mb_detect_encoding("a", "NOPE"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { mb_detect_encoding("a", []); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(mb_check_encoding(["a", "\xff"]), mb_check_encoding(["\xff" => "a"]), mb_check_encoding(["a" => [1, 2.5, null, true, "ok"]]));
try { mb_check_encoding(new stdClass); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }

foreach ([0, 3, 4, 5, -1, -3, -10] as $o) {
    $m = 'x';
    $r = preg_match('/a./', 'xaba', $m, 0, $o);
    echo $o, ' ', var_export($r, true), ' ', json_encode($m), ' ', preg_last_error(), ' ';
    $m = 'x';
    $r = preg_match_all('/a/', 'xaba', $m, PREG_OFFSET_CAPTURE, $o);
    echo var_export($r, true), ' ', json_encode($m), ' ', preg_last_error_msg(), "\n";
}
try { preg_match('/a/', 'a', $m, 0, PHP_INT_MIN); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { array_fill(0, PHP_INT_MAX, 0); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
