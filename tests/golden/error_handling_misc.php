<?php
// set_error_handler inside a running handler applies to later errors there;
// E_STRICT's deprecation; sort() comparison notices reach the handler;
// by-reference array parameters are type-checked; error_log / html_errors.
$outer = function ($l, $m) {
    echo "outer: $m\n";
    set_error_handler(function ($l, $m) { echo "nested: $m\n"; return true; });
    trigger_error('after set in handler', E_USER_NOTICE);
    return true;
};
set_error_handler($outer);
trigger_error('first', E_USER_NOTICE);
trigger_error('second', E_USER_NOTICE);
restore_error_handler();
ob_start(); trigger_error('third', E_USER_NOTICE); echo str_replace(__FILE__, 'F', ob_get_clean());
set_error_handler(function ($l, $m) { echo "[$l] $m\n"; return true; });
echo E_STRICT, "\n";
$o = new stdClass; $a = [$o, 1]; sort($a); echo count($a), "\n";
set_error_handler(function ($l, $m, $f, $n) { throw new ErrorException($m, 0, $l, $f, $n); });
try { $a = [$o, 1]; sort($a); echo "sorted\n"; } catch (ErrorException $e) { echo "caught: ", $e->getMessage(), "\n"; }
try { usort($undef, fn() => 0); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { $s = 'x'; array_push($s, 1); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
restore_error_handler(); restore_error_handler(); restore_error_handler();
$log = sys_get_temp_dir() . '/rhp_errlog_' . getmypid() . '.log';
ini_set('log_errors', '1'); ini_set('display_errors', '0'); ini_set('error_log', $log);
trigger_error('logged', E_USER_WARNING);
echo str_replace(__FILE__, 'F', preg_replace('/^\[[^\]]+\] /', '[date] ', file_get_contents($log)));
unlink($log);
ini_set('log_errors', '0'); ini_set('display_errors', '1'); ini_set('html_errors', '1');
ob_start(); trigger_error('html <b>', E_USER_WARNING); $h = ob_get_clean();
echo str_replace(__FILE__, 'F', $h);
$m = fopen('php://memory', 'r'); var_dump(ftruncate($m, 5), stream_get_contents($m));
