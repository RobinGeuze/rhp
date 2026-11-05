<?php
echo mb_strlen("héllo"), " ", strlen("héllo"), "\n";
echo mb_substr("héllo wörld", 6, 5), "\n";
echo mb_strtoupper("café"), "\n";
echo mb_strtolower("CAFÉ"), "\n";
echo mb_convert_case("hello world", MB_CASE_TITLE), "\n";
echo mb_ord("€"), "\n";
echo mb_chr(8364), "\n";
var_dump(mb_strpos("héllo", "llo"));
echo implode("|", mb_str_split("héllo")), "\n";
