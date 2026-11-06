<?php
// SWF: a negative frame extent wraps as C's unsigned long does.
function swf(int $xmin, int $xmax, int $ymin, int $ymax): string {
    $bits = sprintf('%05b', 15);
    foreach ([$xmin, $xmax, $ymin, $ymax] as $v) {
        $bits .= sprintf('%015b', $v);
    }
    $bits = str_pad($bits, 32 * 8, '0');
    $rect = '';
    foreach (str_split($bits, 8) as $byte) {
        $rect .= chr(bindec($byte));
    }
    return "FWS\x06\x00\x01\x00\x00" . $rect;
}
var_dump(getimagesizefromstring(swf(0, 2000, 0, 400)));
var_dump(getimagesizefromstring(swf(100, 0, 0, 40)));
// The notice prints the data as a C string: up to its first NUL.
set_error_handler(function ($no, $msg) { echo "notice: $msg\n"; return true; });
var_dump(getimagesizefromstring("ab"));
var_dump(getimagesizefromstring("a\0b"));
var_dump(getimagesizefromstring("\0\0"));
