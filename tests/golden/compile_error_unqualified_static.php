<?php
// php 8.5.12: `namespace\static` / `\static` are "Type declaration 'static'
// must be unqualified" compile errors (E_COMPILE_ERROR): fatal, not
// catchable, `finally` and destructors are skipped, shutdown functions run.
ob_start(function ($s) { return str_replace(__FILE__, 'F', $s); });
register_shutdown_function(function () {
    $e = error_get_last();
    echo "shutdown: ", $e['type'], " ", $e['message'], " @", $e['line'], " ", str_replace(__FILE__, 'F', $e['file']), "\n";
});
class D { function __destruct() { echo "dtor\n"; } }
function load() {
    $d = new D;
    eval("namespace X;\nclass A {\n  function f(): namespace\\static {}\n}");
}
echo "before\n";
try {
    load();
} catch (Throwable $t) {
    echo "caught ", get_class($t), "\n";
} finally {
    echo "finally\n";
}
echo "after\n";
