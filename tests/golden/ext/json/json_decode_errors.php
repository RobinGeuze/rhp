<?php
// json_decode() error codes: a container closed by the wrong bracket is a
// state mismatch, a NUL byte outside a string a control character; with
// JSON_THROW_ON_ERROR json_last_error() keeps its old value.
foreach (['{]', '[}', '{"a":1]', '[1,2}', "[1\x00]", "\x00", "1\x00", "{\"a\"\x00:1}", '[[1}]', '{"a":[}}', "[\"a\x01\"]", '[1 2]', '{1:2}', ''] as $j) {
    echo json_encode($j), ': ';
    var_dump(json_decode($j), json_last_error(), json_validate($j), json_last_error());
}
json_decode('[');
try {
    json_decode('{]', false, 512, JSON_THROW_ON_ERROR);
} catch (JsonException $e) {
    echo $e->getMessage(), ' ', $e->getCode(), "\n";
}
var_dump(json_last_error());
try {
    json_encode(NAN, JSON_THROW_ON_ERROR);
} catch (JsonException $e) {
    echo $e->getMessage(), ' ', $e->getCode(), "\n";
}
var_dump(json_last_error());
var_dump(json_encode(NAN, JSON_THROW_ON_ERROR | JSON_PARTIAL_OUTPUT_ON_ERROR), json_last_error());
foreach ([0, -1, PHP_INT_MAX] as $depth) {
    try {
        var_dump(json_decode('[1]', true, $depth));
    } catch (ValueError $e) {
        echo $e->getMessage(), "\n";
    }
}

// invalid UTF-8: one byte at a time in a string, an error outside one
foreach (['"abc', '["a\\', "\xff", "[\xff]", "[1}\xff", "\"a\xffb\"", "\xc3\xa9", "[\"\xc3\xa9\"]", "\"\xe0\x80\x80\"", "\"\xed\xa0\x80\"", "[1,\x01]", "{\"a\xff\":1}", "\"a\xc3\"", 'nul', "nul\xff", "1\xff", "[\"\xef\xbf\xbd\"]"] as $j) {
    echo bin2hex($j), ': ';
    var_dump(json_decode($j), json_last_error());
    var_dump(json_decode($j, true, 512, JSON_INVALID_UTF8_IGNORE), json_last_error());
    var_dump(json_decode($j, true, 512, JSON_INVALID_UTF8_SUBSTITUTE), json_last_error());
    var_dump(json_validate($j), json_last_error(), json_validate($j, 512, JSON_INVALID_UTF8_IGNORE), json_last_error());
}

// The depth counts each container when it opens, empty or not.
foreach (['[]', '[1]', '[[]]', '[{}]', '[[}', "[[\xff", "{\"\xff\":1}"] as $j) {
    foreach ([1, 2, 3] as $d) {
        json_decode($j, true, $d);
        echo bin2hex($j), " $d: ", json_last_error(), " ";
        json_validate($j, $d);
        echo json_last_error(), "\n";
    }
}
