<?php
var_dump($setByA ?? 'unset');
function readGlobal() { global $setByA; return $setByA; }
var_dump(readGlobal());
function incInFunction() {
    $local = 'L';
    include __DIR__ . '/include_global_scope_c.inc';
    return $fromC ?? 'missing';
}
var_dump(incInFunction());
