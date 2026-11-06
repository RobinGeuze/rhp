<?php
set_error_handler(function ($no, $msg) { echo "[$no] $msg\n"; return true; });
// metaphone, quoted-printable, uuencode, hebrev, strtr, charmask ranges.
foreach (["Thompson", "knight", "Schmidt", "Xavier", "wheel", "aero", "ghost", "dumb", "Science", "laugh", "Chris", "", "123"] as $w) {
    echo $w, " => ", metaphone($w), " ", metaphone($w, 2), "\n";
}
$qp = quoted_printable_encode(str_repeat("é ab=c\t", 12) . "\r\nend \r\n");
echo $qp, "\n";
var_dump(quoted_printable_decode($qp) === str_repeat("é ab=c\t", 12) . "\r\nend \r\n");
var_dump(quoted_printable_decode("a=4"), quoted_printable_decode("a= \r\nb=\nc=zz"));
$uu = convert_uuencode(str_repeat("The quick brown fox ", 5));
echo $uu;
var_dump(convert_uudecode($uu) === str_repeat("The quick brown fox ", 5));
var_dump(convert_uuencode(""), convert_uuencode("a"), @convert_uudecode("!!!"));
var_dump(bin2hex(hebrev("\xe0\xe1\xe2 abc (\xe3) def", 6)));
var_dump(strtr("hello world", ["hello" => "bye", "h" => "H", "o" => 0, "world" => 1.5]));
var_dump(strtr("aaa", ["a" => "b", "aa" => "c"]), strtr("abc", "aab", "xyz"), strtr("abc", []), strtr("", ["" => "x"]));
var_dump(trim("..abc..", "."), trim("abcxyz", "a..c"), ltrim("abc", "c..a"), rtrim("abc..", "z.."), ucwords("hello-world foo", "-"));
var_dump(str_word_count("'tis -the- end-", 1), str_word_count("a1b2", 2, "1..2"));
var_dump(substr_count("aaaa", "aa", -3, -1), substr_compare("abcde", "cdx", 2), substr_compare("ab", "abc", 0, 5));
