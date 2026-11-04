<?php
var_dump(ini_set('precision', 5)); echo 1/3, ' ', 1e10/3, ' ', 0.1+0.2, "\n";
var_dump(ini_set('precision', '-1')); echo 1/3, ' ', 0.1+0.2, ' ', 1e15, ' ', 1e17, "\n";
var_dump(ini_set('precision', '0')); echo 1/3, ' ', 15.5, "\n";
var_dump(ini_set('precision', '-5'), ini_get('precision'));
var_dump(ini_set('precision', 'abc'), ini_get('precision')); echo 2/3, "\n";
ini_restore('precision'); echo 1/3, ' ', ini_get('precision'), "\n";
var_dump(ini_set('serialize_precision', '5')); echo json_encode(0.1+0.2), ' ', var_export(1/3, true), ' ', serialize(0.1), "\n"; var_dump(1/3);
var_dump(ini_set('serialize_precision', '17')); var_dump(0.1); echo json_encode([0.1, 1e15]), "\n";
var_dump(ini_set('serialize_precision', '-2'), ini_get('serialize_precision'));
ini_set('serialize_precision', '-1'); var_dump(1e15, 1e16, 1e17, 0.1 + 0.2); echo json_encode([1e15, 1e16, 1e17]), "\n";
var_dump(ini_set('arg_separator.output', ';')); echo http_build_query(['a' => 1, 'b' => [1, 2]]), ' ', http_build_query(['a' => 1, 'b' => 2], '', '|'), "\n";
var_dump(ini_set('arg_separator.output', ''), ini_get('arg_separator.output'));
var_dump(ini_set('error_reporting', E_ALL), error_reporting(), ini_get('error_reporting'));
var_dump(error_reporting(0), ini_get('error_reporting'), error_reporting(E_ALL & ~E_NOTICE), ini_get('error_reporting'));
var_dump(ini_set('error_reporting', 'abc'), error_reporting(), error_reporting(null), error_reporting('5'), error_reporting());
try { error_reporting('x'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
var_dump(set_time_limit(5), ini_get('max_execution_time'), set_time_limit(0), ini_get('max_execution_time'));
foreach (['abc', '-5', '1G', '2M', '1M', '3M', '0x400000', '64m', ' 64M ', '4096K', '8388608', '1.5G', '64MB', '0', '', '-1', true, 4194304, null, 1.5] as $v) {
    echo var_export($v, true), ' => ', var_export(@ini_set('memory_limit', $v), true), ' now=', ini_get('memory_limit'), "\n";
}
foreach (['-1', '0', '1', '2', '-1', 'x'] as $v) {
    echo 'zend.assertions ', $v, ' => ', var_export(@ini_set('zend.assertions', $v), true), ' now=', ini_get('zend.assertions'), "\n";
}
var_dump(@ini_set('date.timezone', 'Bogus/Zone'), ini_get('date.timezone'), ini_set('date.timezone', 'Europe/Paris'), date_default_timezone_get());
try { ini_set('precision', []); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { ini_set('precision', new stdClass); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
var_dump(ini_set('no.such.setting', '1'));
