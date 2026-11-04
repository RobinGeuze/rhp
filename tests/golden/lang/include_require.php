<?php
// require/include of a function and class library, a file returning an
// array with a closure, require_once seeing a path with `..` as the same
// file, and a suppressed include of a missing file.
require_once __DIR__ . '/include_require_lib.inc';
echo greet('x'), '|', (new W)->go(), '|', V, "\n";
$c = require __DIR__ . '/include_require_config.inc';
echo $c['n'], $c['s'], ($c['f'])(2), "\n";
require_once __DIR__ . '/include_require_lib.inc'; echo "dedup-ok\n";
$r = require __DIR__ . '/include_require_sub/relay.inc'; echo $r, "\n";
$m = @include __DIR__ . '/nope.php'; var_dump($m);
echo 'file-ok=', (str_contains(__FILE__, 'include_require.php') ? 'yes' : 'no'), "\n";
