<?php
// php_verror's manual link: html_errors with docref_root (and docref_ext).
set_error_handler(function ($no, $msg) {
    if (str_contains($msg, 'Failed to open stream')) {
        echo $msg, "\n";
    }
    return true;
});
include 'no-such-file-here.php';
ini_set('html_errors', '1');
include 'no-such-file-here.php';
ini_set('docref_root', 'https://php.example/manual/');
include 'no-such-file-here.php';
ini_set('docref_ext', '.html');
include_once 'no-such-file-here.php';
