<?php
// zend_compile_declare and the parser's encoding checks, read back with
// error_get_last() (compile warnings skip error handlers).
$cases = [
    'declare(foo=1);',
    'declare(Ticks=1);',
    'declare(encoding="UTF-8");',
    'declare(encoding="no-such-encoding");',
    'declare(strict_types=1, bar=2);',
];
foreach ($cases as $code) {
    error_clear_last();
    @eval($code);
    $e = error_get_last();
    echo $code, ' => ', $e ? $e['type'] . ' ' . $e['message'] : 'nothing', "\n";
}
