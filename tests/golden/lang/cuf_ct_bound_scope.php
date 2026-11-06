<?php
// call_user_func[_array] with a string literal naming an internal function
// is bound at compile time: the callee's call is not dynamic.
function d() {
    var_dump(call_user_func('func_get_args'));
    var_dump(call_user_func_array('func_num_args', []));
    var_dump(call_user_func('Func_Get_Arg', 1));
    $k = 'v';
    var_dump(call_user_func('compact', 'k'));
    var_dump(call_user_func_array('get_defined_vars', []));
    $n = 'func_get_args';
    try { call_user_func($n); } catch (Error $e) { echo $e->getMessage(), "\n"; }
    try { call_user_func('\func_get_args'); } catch (Error $e) { echo $e->getMessage(), "\n"; }
    $c = 'call_user_func';
    try { $c('func_get_args'); } catch (Error $e) { echo $e->getMessage(), "\n"; }
    try { array_map('func_get_args', [1]); } catch (Error $e) { echo $e->getMessage(), "\n"; }
}
d(1, 2);
try { call_user_func('func_get_args'); } catch (Error $e) { echo $e->getMessage(), "\n"; }
