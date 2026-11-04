<?php
$a = [1, 2, 3];
echo @$a[10] ?? 'default', "\n";
$arr = [];
$arr['x']['y'] = 1;  // autovivify
print_r($arr);
echo $undefined ?? "undef", "\n";
$obj = new stdClass();
$obj->dynamic = 5;
echo $obj->dynamic, "\n";
$str = "abc";
echo $str[0] . $str[-1], "\n";
$n = null;
echo $n + 5, "\n";
echo strlen($n ?? ''), "\n";
$config = ['db' => ['host' => 'localhost']];
echo $config['db']['port'] ?? 3306, "\n";
echo isset($config['cache']) ? 'set' : 'notset', "\n";
echo empty($config['db']) ? 'empty' : 'nonempty', "\n";
var_dump(array_key_exists('db', $config));
