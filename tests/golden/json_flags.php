<?php
echo json_encode(['a'=>1.0, 'b'=>-2.0, 'c'=>1.5, 'd'=>1e25, 'e'=>0.0], JSON_PRESERVE_ZERO_FRACTION), "\n";
var_dump(json_encode("\xffa", JSON_INVALID_UTF8_SUBSTITUTE), json_encode("\xffa\xc3", JSON_INVALID_UTF8_IGNORE), json_encode("\xffa", JSON_INVALID_UTF8_SUBSTITUTE|JSON_UNESCAPED_UNICODE), json_encode(["\xff" => 1], JSON_INVALID_UTF8_IGNORE));
try { json_decode('{', false, 512, JSON_THROW_ON_ERROR); } catch (JsonException $e) { echo get_class($e), $e->getCode(), $e->getMessage(), "\n"; }
try { json_encode(NAN, JSON_THROW_ON_ERROR); } catch (JsonException $e) { echo $e->getCode(), $e->getMessage(), "\n"; }
var_dump(json_decode('{"\u0000a":1}'), json_last_error(), json_last_error_msg(), json_decode('{"\u0000a":1}', true));
