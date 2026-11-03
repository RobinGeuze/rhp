<?php
// zend_ini_parser's Bison automaton decides and words INI syntax errors.
set_error_handler(function ($no, $str) {
    echo "warning: ", json_encode($str), "\n";
    return true;
});
$cases = [
    "a=b\nc=\${", "[sec\nx=1", "a = (1", "=x", "a=\"x", "a=1\nb=)\n", "[a]\nb=1\n[c\n",
    "x[=1", "a=\${b", "a = 1 |", "a=b=c", "[x]y=1\nz=)", "a=\"line1\nline2\" )",
    "a = 'x' 'y'", "a = 1\n\n\n}=2", "a=1 ; comment\nb=(", "x = \${a:-b",
];
foreach ($cases as $c) {
    foreach ([INI_SCANNER_NORMAL, INI_SCANNER_RAW, INI_SCANNER_TYPED] as $mode) {
        var_dump(parse_ini_string($c, true, $mode));
    }
}
