<?php
// The notices / warnings of the output-control functions: no buffer to
// act on, non-removable / non-flushable / non-cleanable buffers, invalid
// ob_start() callbacks, and argument type errors.
set_error_handler(function ($no, $msg) {
    echo "[E$no] $msg\n";
    return true;
});
var_dump(ob_flush(), ob_clean(), ob_end_flush(), ob_end_clean(), ob_get_flush(), ob_get_clean());
var_dump(ob_get_contents(), ob_get_length(), ob_get_level(), ob_list_handlers(), ob_get_status());

class C { static function f($s) { return $s; } private function priv($s) { return $s; } }
foreach ([1.5, 'nonexistent_fn', ['NoSuchClass', 'f'], ['C', 'nope'], 'C::nope', [new C], [new C, 'priv'], [1, 2]] as $cb) {
    var_dump(ob_start($cb));
}
var_dump(ob_get_level());

foreach (['chunk_size' => [null, 'abc'], 'flags' => [null, 0, []]] as $what => $values) {
    foreach ($values as $v) {
        try {
            $args = $what === 'chunk_size' ? [null, $v] : [null, 0, $v];
            var_dump(ob_start(...$args));
            ob_end_clean();
        } catch (TypeError $e) {
            echo get_class($e), ": ", $e->getMessage(), "\n";
        }
    }
}
try {
    ob_get_status([]);
} catch (TypeError $e) {
    echo $e->getMessage(), "\n";
}
// negative chunk sizes are treated as 0
ob_start(null, -5);
$st = ob_get_status();
ob_end_clean();
echo "chunk_size=", $st['chunk_size'], " buffer_size=", $st['buffer_size'], "\n";
ob_start(null, 5000);
echo str_repeat("x", 3);
$st = ob_get_status();
ob_end_clean();
echo "chunk_size=", $st['chunk_size'], " buffer_size=", $st['buffer_size'], "\n";
// buffer growth as reported by ob_get_status
ob_start();
$sizes = [];
foreach ([100, 16284, 1, 20000, 70000] as $n) {
    echo str_repeat("y", $n);
    $sizes[] = ob_get_status()['buffer_size'];
}
ob_end_clean();
echo implode(",", $sizes), "\n";

// non-flushable / non-cleanable / non-removable, by name and level
ob_start();
ob_start('C::f', 0, 0);
echo "x";
var_dump(ob_flush(), ob_clean(), ob_end_flush(), ob_end_clean(), ob_get_flush(), ob_get_clean());
var_dump(ob_get_level());
