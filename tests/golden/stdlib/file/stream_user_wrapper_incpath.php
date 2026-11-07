<?php
// include of a plain name through include_path entries of a user wrapper:
// a failed open names the resolved path (zend_stream_open gets it).
class IncPathWrapper {
    public $context;
    function url_stat($p, $f) { echo "url_stat $p $f\n"; return ['mode' => 0100644]; }
    function stream_open($p, $m, $o, &$op) { echo "stream_open $p\n"; return false; }
}
stream_wrapper_register('ipw', 'IncPathWrapper');
set_error_handler(function ($no, $msg) { echo $msg, "\n"; return true; });
set_include_path('ipw://lib');
var_dump(include 'one.php');
var_dump(include_once 'two.php');
var_dump(include 'ipw://lib/three.php');
