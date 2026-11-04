<?php
$o = new stdClass();
$o->name = "Bob"; $o->age = 30;
echo $o->name, " is ", $o->age, "\n";
var_dump($o instanceof stdClass);
echo get_class($o), "\n";

$obj = (object)['x' => 1, 'y' => 2, 'z' => 3];
echo $obj->x + $obj->y + $obj->z, "\n";
foreach ($obj as $k => $v) echo "$k=$v ";
echo "\n";

echo ((object)"hi")->scalar, "\n";
var_dump((object)null instanceof stdClass);

$arr = (array)(object)['a' => 1, 'b' => 2];
print_r($arr);

echo count(get_object_vars($o)), "\n";

$rec = new stdClass();
$rec->items = [1, 2, 3];
$rec->meta = (object)['count' => 3];
echo json_encode($rec), "\n";
$dec = json_decode('{"name":"Al","nested":{"x":1}}');
echo get_class($dec), " ", $dec->name, " ", $dec->nested->x, "\n";
echo json_encode(json_decode('{"a":1,"b":[2,3]}')), "\n";
