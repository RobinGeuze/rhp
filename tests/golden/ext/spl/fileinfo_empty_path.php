<?php
// php_stat: an empty name is false, with no error.
$f = new SplFileInfo('');
foreach (['getSize', 'getMTime', 'getType', 'getPerms', 'isFile'] as $m) {
    try { var_dump($f->$m()); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
}
