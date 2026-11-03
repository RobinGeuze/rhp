<?php
// memory_hard_limit_percent (new in php 8.5.12): registered with a NULL
// default; values below 100 other than -1 warn and are refused, malformed
// quantities warn as zend_ini_parse_quantity_warn does.
set_error_handler(function ($no, $msg) {
    echo "warning: $msg\n";
    return true;
});
$all = ini_get_all();
var_dump($all['memory_hard_limit_percent']);
var_dump(ini_get_all(null, false)['memory_hard_limit_percent']);
var_dump(ini_get('memory_hard_limit_percent'));
foreach (['50', '150', '-1', 'abc', '-2', '', '100', '1e3', '200abc', ' 99', '0x100', '1k', null, 99, 100.0, true] as $v) {
    echo var_export($v, true), ' => ';
    var_dump(ini_set('memory_hard_limit_percent', $v));
}
var_dump(ini_get('memory_hard_limit_percent'));
ini_restore('memory_hard_limit_percent');
var_dump(ini_get('memory_hard_limit_percent'));
