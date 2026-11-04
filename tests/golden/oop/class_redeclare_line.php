<?php
// The early-bound class is declared at the `class` keyword's line (not
// its attributes' or modifiers'); the conditional one then collides.
ini_set('display_errors', '0');
register_shutdown_function(function () {
    $e = error_get_last();
    echo $e['type'], ': ', str_replace(__DIR__ . '/', '', $e['message']), "\n";
    echo basename($e['file']), ':', $e['line'], "\n";
});
require __DIR__ . '/class_redeclare_line.inc';
