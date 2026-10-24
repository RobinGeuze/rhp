<?php
$s = "Héllo Wörld café";
echo strlen($s), "\n";
echo mb_strlen($s), "\n";
echo mb_strtoupper($s), "\n";
echo mb_strtolower("ÀÉÎ"), "\n";
echo mb_substr($s, 0, 5), "\n";
echo mb_strpos($s, "W"), "\n";
$emoji = "a😀b";
echo strlen($emoji), " ", mb_strlen($emoji), "\n";
echo mb_str_split("héllo")[1], "\n";
echo mb_convert_case("hello world", MB_CASE_TITLE), "\n";
$jp = "日本語";
echo mb_strlen($jp), "\n";
echo mb_substr($jp, 1, 1), "\n";
var_dump(mb_detect_encoding("plain"));
