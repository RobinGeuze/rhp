<?php
// php_json_escape_string: the JSON_HEX_* flags, U+2028/U+2029, invalid UTF-8.
$s = "a&b<c>'d\"e/f\\g";
foreach ([0, JSON_HEX_TAG, JSON_HEX_AMP, JSON_HEX_APOS, JSON_HEX_QUOT, 15, 15 | JSON_UNESCAPED_SLASHES] as $f) {
    echo $f, ' ', json_encode($s, $f), "\n";
}
echo json_encode(['<k>' => "&'"], JSON_HEX_TAG | JSON_HEX_AMP | JSON_HEX_APOS), "\n";
echo json_encode(['<a>', '1e3', '12'], JSON_HEX_TAG | JSON_NUMERIC_CHECK | JSON_PRETTY_PRINT), "\n";

// line terminators
$lt = "x\u{2028}y\u{2029}z\u{e9}\u{1F600}";
var_dump(JSON_UNESCAPED_LINE_TERMINATORS);
echo json_encode($lt), "\n";
echo json_encode($lt, JSON_UNESCAPED_UNICODE), "\n";
echo json_encode($lt, JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_LINE_TERMINATORS), "\n";
echo json_encode($lt, JSON_UNESCAPED_LINE_TERMINATORS), "\n";

// control characters and DEL
echo json_encode("\x00\x01\x08\x0b\x0c\x1f\x7f"), "\n";

// invalid UTF-8: php's decoder decides how many bytes one error covers
foreach (["a\xe0\x80\x80b", "a\xc3b", "a\xf0\x9f\x98b", "a\xed\xa0\x80b", "a\xff\xfeb", "\xc0\xaf"] as $bad) {
    var_dump(json_encode($bad));
    var_dump(json_last_error_msg());
    var_dump(json_encode($bad, JSON_INVALID_UTF8_SUBSTITUTE));
    var_dump(json_encode($bad, JSON_INVALID_UTF8_SUBSTITUTE | JSON_UNESCAPED_UNICODE));
    var_dump(json_encode($bad, JSON_INVALID_UTF8_IGNORE));
}

// JSON_PARTIAL_OUTPUT_ON_ERROR: a bad string is null, a bad key ""
var_dump(json_encode("a\xff", JSON_PARTIAL_OUTPUT_ON_ERROR), json_last_error());
var_dump(json_encode(["x" => "a\xff", "y" => 1], JSON_PARTIAL_OUTPUT_ON_ERROR), json_last_error());
var_dump(json_encode(["a\xff" => 1, "b" => 2], JSON_PARTIAL_OUTPUT_ON_ERROR), json_last_error());
var_dump(json_encode(["a\xff" => 1, "b" => 2], JSON_PARTIAL_OUTPUT_ON_ERROR | JSON_PRETTY_PRINT));
$o = new stdClass;
$o->{"p\xff"} = 1;
$o->q = "\xfe";
var_dump(json_encode($o, JSON_PARTIAL_OUTPUT_ON_ERROR), json_last_error());
// a bad key without the flag fails the call, and a later error wins
var_dump(json_encode(["a\xff" => 1]), json_last_error());
var_dump(json_encode(["a\xff" => NAN]), json_last_error());
try {
    json_encode(["a\xff" => 1], JSON_THROW_ON_ERROR);
} catch (JsonException $e) {
    echo get_class($e), ': ', $e->getMessage(), ' ', $e->getCode(), "\n";
}
