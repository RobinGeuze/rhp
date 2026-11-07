<?php
// After a multi-line closure, php reports what follows in the statement at
// the closure's last line (zend_compile_func_decl leaves the line there);
// a call keeps its own line.
set_error_handler(function ($no, $msg, $file, $line) {
    echo "$msg at line $line\n";
    return true;
});
echo (function () {
    return [];
})() . "\n";
$a = [function () {
    return 1;
}, $undef1];
$s = (fn () =>
    1)() . $undef2;
try {
    array_map(function ($x) {
        return $x;
    }, 5);
} catch (TypeError $e) {
    echo $e->getLine(), "\n";
}
$x = (function () {
    return null;
})()?->foo . $undef3;
