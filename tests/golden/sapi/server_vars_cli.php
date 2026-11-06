<?php
// php-cli's own $_SERVER entries come after the environment, in this
// order.
$k = array_keys($_SERVER);
echo implode(',', array_slice($k, -9)), "\n";
var_dump($_SERVER['DOCUMENT_ROOT']);
var_dump(basename($_SERVER['PATH_TRANSLATED']) === basename(__FILE__));
var_dump(is_float($_SERVER['REQUEST_TIME_FLOAT']));
var_dump($_SERVER['REQUEST_TIME'] === (int) $_SERVER['REQUEST_TIME_FLOAT']);
