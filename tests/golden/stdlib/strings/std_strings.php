<?php
echo sprintf("%05d %.2f %s %x\n", 42, 3.14159, "hi", 255);
echo sprintf("%-10s|%+d|%'*8s\n", "left", 7, "pad");
echo substr("Hello World", 6), "\n";
echo substr("Hello World", -5, 3), "\n";
echo str_replace(["a", "b"], ["X", "Y"], "abcabc"), "\n";
echo str_pad("5", 4, "0", STR_PAD_LEFT), "\n";
echo number_format(1234567.891, 2), "\n";
echo ucwords("hello world foo"), "\n";
echo strrev("abc"), " ", strpos("hello", "l"), "\n";
var_dump(str_starts_with("foobar", "foo"), str_ends_with("foobar", "baz"));
