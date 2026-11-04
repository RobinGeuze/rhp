<?php
$data = [
    'name' => 'test',
    'values' => [1, 2, 3],
    'nested' => ['a' => true, 'b' => null, 'c' => 3.14],
    'unicode' => 'café',
    'empty' => [],
];
echo json_encode($data), "\n";
echo json_encode($data, JSON_PRETTY_PRINT), "\n";
$json = '{"x":1,"y":[true,false,null],"z":"hi"}';
$decoded = json_decode($json, true);
print_r($decoded);
var_dump(json_decode('123'));
var_dump(json_decode('"string"'));
var_dump(json_decode('[1,2.5,true]', true));
$obj = json_decode('{"a":1,"b":2}');
echo $obj->a + $obj->b, "\n";
echo json_encode(["slash" => "a/b", "quote" => 'he said "hi"']), "\n";
echo json_encode([1.0, 2.5, 100]), "\n";
