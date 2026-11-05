<?php
// Calls whose earlier parameters php parses as a resource, a stream or any
// value (Z_PARAM_RESOURCE, PHP_Z_PARAM_STREAM, Z_PARAM_ZVAL): the scalar
// parameters after them are parsed in the caller's mode, and a resource
// error comes first where php's parse checks it.
set_error_handler(function ($no, $msg) {
    echo $no === E_DEPRECATED ? "Deprecated" : "Warning", ": $msg\n";
    return true;
});
function t(callable $c) {
    try {
        $r = $c();
        echo is_resource($r) ? "resource" : var_export($r, true), "\n";
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
$fp = fopen("php://memory", "w+");
$closed = fopen("php://memory", "w+");
fclose($closed);
$ctx = stream_context_create();
$proc = proc_open("true", [], $pipes);

t(fn() => fwrite($fp, null));
t(fn() => fwrite($fp, 5));
t(fn() => fwrite($fp, []));
t(fn() => fwrite($fp, "x", "1"));
t(fn() => fwrite($closed, []));
t(fn() => fwrite($ctx, null));
t(fn() => fwrite(1, []));
t(fn() => fwrite(null, "x"));
t(fn() => fread($fp, "abc"));
t(fn() => fread($fp, "5"));
t(fn() => fread($fp, null));
t(fn() => fread($fp, 1.5));
t(fn() => fread($fp, true));
t(fn() => fseek($fp, "1x"));
t(fn() => fseek($fp, 0, null));
t(fn() => ftruncate($fp, "2"));
t(fn() => stream_get_contents($fp, null, null));
t(fn() => stream_get_contents($fp, "x"));
t(fn() => fgets($fp, "z"));
t(fn() => stream_set_blocking($fp, "yes"));
t(fn() => file_get_contents(__FILE__, false, 1));
t(fn() => file_get_contents(__FILE__, false, $fp, "x"));
t(fn() => file_get_contents(__FILE__, false, null, 0, "1"));
t(fn() => proc_close(1));
t(fn() => proc_close(null));
t(fn() => proc_terminate($fp, "x"));
t(fn() => proc_terminate($proc, "x"));
t(fn() => get_resource_type(1));
t(fn() => closedir(1));
t(fn() => count([1, 2], null));
t(fn() => count([1, 2], "x"));
t(fn() => count(5, "x"));
t(fn() => ob_start(null, "x"));
t(fn() => ob_start(null, null));
t(fn() => hash_equals(1, "x"));
t(fn() => hash_equals("x", null));
t(fn() => hash_equals("x", "x"));
t(fn() => array_key_exists([], [], 1));
t(fn() => array_key_exists("a", "b"));
proc_close($proc);
