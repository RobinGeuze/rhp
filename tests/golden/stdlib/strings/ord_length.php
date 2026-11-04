<?php
// php 8.5 deprecates ord() of a string that is not one byte long; a
// one-byte literal is folded at compile time.
set_error_handler(function ($no, $msg) {
    echo "Deprecated: $msg\n";
    return true;
});
$empty = "";
$two = "ab";
var_dump(ord("a"), ord("ab"), ord(""), ord($empty), ord($two), ord(7), ord(12));
