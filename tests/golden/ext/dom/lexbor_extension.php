<?php
var_dump(extension_loaded('lexbor'), phpversion('lexbor'), get_extension_funcs('lexbor'));
$e = get_loaded_extensions();
var_dump(array_search('lexbor', $e) === array_search('date', $e) + 1);
ob_start(); phpinfo(INFO_MODULES); $i = ob_get_clean();
preg_match('/^lexbor\n\n(.*?)\n\n/ms', $i, $m); echo $m[1], "\n";
