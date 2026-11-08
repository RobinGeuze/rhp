<?php
var_dump(function_exists('router_fn'), $fromrouter ?? null, basename(getcwd()));
var_dump(php_sapi_name(), $_SERVER['PATH_INFO'] ?? null, $_SERVER['PHP_SELF'], $_GET);
var_dump($_SERVER['HTTP_X_A_B'], $_SERVER['SERVER_PROTOCOL'], ini_get('html_errors'));
