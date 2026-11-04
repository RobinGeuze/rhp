<?php
define('MYCONST', 'constval');
putenv('RHP_INI_TEST=fromenv');

$ini = <<<'INI'
; comment line
[first]
plain = hello world
quoted = "a \"q\" \\ \$ \n x"
single = 'raw $x \n'
num = 42
neg = -7
float = 1.5
hexish = 0x1A
bool_on = on
bool_yes = yes
bool_true = true
bool_off = off
bool_no = no
bool_none = none
bool_false = false
nul = null
empty =
const = MYCONST
constcat = MYCONST and more
colon = a:b
expr_or = 1 | 2
expr_and = 7 & 3
expr_xor = 5 ^ 1
expr_not = ~0
expr_bang = !1
expr_paren = (1 | 4) & 5
trailing = value   ; with comment
path = /usr/local/bin
url = http://example.com/?a
dollar = $notvar
envvar = ${RHP_INI_TEST}
envmissing = ${RHP_INI_MISSING_VAR}
envquoted = "pre ${RHP_INI_TEST} post"

[second]
arr[] = one
arr[] = two
map[key] = v1
map[other] = v2
5 = five
n[3] = three
007[x] = y
mix = "quoted" unquoted
 spaced key = sv
dotted.key = dv

[ 10 ]
a = 1
INI;

foreach ([INI_SCANNER_NORMAL, INI_SCANNER_RAW, INI_SCANNER_TYPED] as $mode) {
    echo "== mode $mode, sections\n";
    var_dump(parse_ini_string($ini, true, $mode));
}
echo "== no sections\n";
var_dump(parse_ini_string($ini));

$cases = [
    "a=1\nb=2",
    "a = \"unterminated",
    "a = b = c",
    "[sec",
    "a[ = 1",
    "key",
    "a=\${x:-b}",
    "a=\${xy:-def}",
    "yes = 1",
    "a = 'x' 'y'",
    "a = \"C:\\path\\\"\nb=2",
    "a=1;c",
    "a=\"x;y\" ; z",
    "\xef\xbb\xbfbom=1",
    "a=1\r\nb=2\rc=3",
    "x = 99999999999999999999",
    "x = 1e3",
    "x = .5",
    "x = 5.",
    "x = 1 2",
    "[a]\nk=1\n[a]\nj=2",
    "a.b c=1",
    "a=\"multi\nline\"",
    "a=\"\${RHP_INI_TEST}\"",
    "a = true story",
    "a = (1",
    "a = ~",
    "a = 1 |",
    "!a=1",
    "a=\$\\{",
];
foreach ($cases as $c) {
    echo json_encode($c), "\n";
    foreach ([INI_SCANNER_NORMAL, INI_SCANNER_RAW, INI_SCANNER_TYPED] as $mode) {
        echo "  $mode: ", json_encode(@parse_ini_string($c, true, $mode)), "\n";
    }
}
var_dump(@parse_ini_string("a=1", false, 7));
var_dump(parse_ini_string(""));
