<?php
// pack()'s first pass: every code's arguments must be there before anything
// is packed; extra arguments warn.
set_error_handler(function ($no, $msg) {
    echo "warning: $msg\n";
    return true;
});
$cases = [
    ['H*'], ['H'], ['h4'], ['a'], ['A*'], ['Z5'], ['N'], ['N2', 1], ['n*'], ['C3', 1, 2],
    ['H3a', 'zz'], ['NH*', 1], ['H*N', 'ab'], ['N', 1, 2], ['H*', 'ab', 'cd', 'ef'],
    ['C*', 1, 2, 3], ['H*', 'zz'], ['H5', 'abc'], ['h*', '1a2b'], ['N0', 5], ['a0'],
    ['v*'], ['J', 1, 2],
];
foreach ($cases as $c) {
    $f = array_shift($c);
    echo $f, ': ';
    try {
        var_dump(bin2hex(pack($f, ...$c)));
    } catch (\ValueError $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}
