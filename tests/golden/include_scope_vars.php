<?php
// An included file's variables live in the including scope even when that
// scope never names them: at top level the global symbol table ($argv,
// variables the include creates), in a function its local by-name table.
$top = sys_get_temp_dir() . '/rhp_inc_top_' . getmypid() . '.php';
$fn = sys_get_temp_dir() . '/rhp_inc_fn_' . getmypid() . '.php';
file_put_contents($top, '<?php echo gettype($argv), "\n"; $counter = ($counter ?? 0) + 1; echo "counter=$counter\n";');
file_put_contents($fn, '<?php $counter = ($counter ?? 0) + 10; $made = "made";');
include $top;
include $top;
function g($fn) {
    include $fn;
    include $fn;
    $v = 'counter';
    $m = 'made';
    return [$$v, $$m];
}
var_dump(g($fn));
unlink($top);
unlink($fn);
