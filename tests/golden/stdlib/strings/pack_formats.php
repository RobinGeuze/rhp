<?php
set_error_handler(function ($no, $msg) { echo "[$no] $msg\n"; return true; });
// pack.c formats and edge cases.
echo bin2hex(pack("nvc*", 0x1234, 0x5678, 65, 66)), "\n";
echo bin2hex(pack("NVJPqQ", 1, 2, 3, 4, -5, 6)), "\n";
echo bin2hex(pack("fgGdeE", 1.5, 1.5, 1.5, -2.25, -2.25, -2.25)), "\n";
echo bin2hex(pack("a5A5Z5a*Z*", "ab", "ab", "abcdefg", "xyz", "q")), "\n";
echo bin2hex(pack("h3H*x2X1@8", "abc", "1f2")), "\n";
echo bin2hex(pack("H3", "12zz")), "\n";
var_dump(unpack("c2chars/nint", "\xff\x01\x00\x02"));
var_dump(unpack("a3x/A4y/Z*z", "ab cd  \0ef\0gh"));
var_dump(unpack("H*hex/h2", "\x12\x34"), unpack("@1/C", "\x01\x02"), unpack("C2/X/C", "\x01\x02"));
var_dump(unpack("q", pack("q", PHP_INT_MIN)), unpack("E", pack("E", 0.1)), unpack("g", pack("g", 3.5)));
var_dump(unpack("N", "abc"), unpack("C*", ""), unpack("C", "ab", 1));
foreach (["y", "N2"] as $f) {
    try { var_dump(pack($f, 1)); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
}
var_dump(pack("x*X*@*"), pack("C", 1, 2));
try { unpack("C", "a", 5); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { unpack("B", "a"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
