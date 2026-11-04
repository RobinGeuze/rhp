<?php
echo mb_strlen("héllo wörld"), " ", mb_strlen("日本語テスト"), "\n";
echo mb_substr("héllo wörld", 2, 4), "\n";
echo mb_strtoupper("héllo grüße"), " | ", mb_strtolower("HÉLLO GRÜßE"), "\n";
echo mb_convert_case("héllo wörld", MB_CASE_TITLE), "\n";
echo implode(",", mb_str_split("日本語")), "\n";
echo var_export(mb_strpos("héllo wörld", "wörld"), true), " ", mb_strwidth("日本語ab"), "\n";
echo mb_ord("日"), " ", mb_chr(26085), " ", mb_ord("é"), "\n";
