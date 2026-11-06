<?php
set_error_handler(function ($no, $msg) { echo "E$no: $msg
"; return true; });
function inc() {
    $outer = 'o';
    include __DIR__ . '/extract_compact_scope.inc';
    var_dump($fromInc, $incLocal);
    var_dump(get_defined_vars());
}
inc();
function after() {
    extract(['late' => 1]);
    echo $late, "\n";
    unset($late);
    echo @$late, "|\n";
    $n = 'late';
    var_dump(isset($$n));
    extract(['late' => 2], EXTR_SKIP);
    var_dump($late);
    $k = 'kk';
    $$k = 1;
    unset($$k);
    var_dump(get_defined_vars());
    $$k = 2;
    var_dump(array_keys(get_defined_vars()));
}
after();
function statics() {
    static $s = 1;
    global $gl;
    extract(['s' => 5, 'gl' => 6]);
    var_dump($s, $GLOBALS['gl']);
}
$gl = 0;
statics(); statics();
function refsGlobals() {
    extract($GLOBALS, EXTR_REFS);
    $gl = 'changed';
}
refsGlobals();
var_dump($gl);
$arr = ['p1' => 1];
extract($arr, EXTR_REFS);
$p1 = 'via ref';
var_dump($arr);
