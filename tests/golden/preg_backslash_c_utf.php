<?php
// GH-21134 (php 8.5.10): \C is forbidden under the `u` modifier.
set_error_handler(function ($no, $str) { echo "E$no: $str\n"; return true; });
var_dump(preg_match("/a\\Cb/u", "axb"));
var_dump(preg_last_error(), preg_last_error_msg());
$r = preg_match_all("/(.*)\\C/u", "\u{e0}", $m);
var_dump($r, $m);
var_dump(preg_replace("/\\C/u", "x", "abc"));
var_dump(preg_split("/\\C/u", "abc"));
// Without `u`, \C still matches one byte.
var_dump(preg_match("/a\\Cb/", "axb"));
var_dump(preg_match_all("/\\C/", "\u{e0}"));
// \\C (a literal backslash then C) is fine under `u`.
var_dump(preg_match("/\\\\C/u", "\\C"));
