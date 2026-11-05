<?php
// Builtin diagnostics: pcre compile errors, unserialize, hex2bin, iconv,
// var_export cycles and __sleep problems.
set_error_handler(function ($l, $m) { echo "[$l] $m\n"; return true; });
var_dump(preg_match('/(/', 'x'), preg_match('', 'x'), preg_match('abc', 'x'), preg_match('/abc', 'x'), preg_match('(abc', 'x'), preg_match('/a/z', 'x'));
var_dump(preg_match('  /a/', 'a'), preg_match('/a/b/', 'a'), preg_match('/a/ i', 'A'));
var_dump(preg_replace('/(/', 'x', 'y'), preg_replace_callback('/[/', fn($m) => '', 'y'), preg_split('/(/', 'y'), preg_grep('/(/', ['y']));
var_dump(preg_last_error_msg());
foreach (['garbage', 'i:5', 'i:5;x', 'a:1:{i:0;}', 'a:1:{i:0;i:1;', 's:5:"abc";', 's:9:"abc";', 'a:2:{i:0;i:1;i:1;zz}', 'O:8:"stdClass":1:{s:1:"a";}', 'b:2;', '', 'E:3:"X:Y";'] as $s) {
    var_dump(unserialize($s));
}
var_dump(hex2bin('abc'), hex2bin('zz'));
var_dump(iconv('UTF-8', 'ASCII', "\xc3\xa9"), iconv('NOPE', 'ASCII', 'a'), iconv('UTF-8', 'ASCII//IGNORE', "a\xc3\xa9"), iconv('UTF-8', 'UTF-8', "a\xc3"), bin2hex(iconv('UTF-8', 'ISO-8859-1', "\xc3\xa9")));
$o = new stdClass; $o->c = $o; var_export($o); echo "\n";
class S { public $a = 1; function __sleep() { return ['a', 'nonexist', 'a']; } }
echo serialize(new S), "\n";
class S2 { function __sleep() { return 5; } }
echo serialize(new S2), "\n";
