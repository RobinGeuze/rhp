<?php
var_dump(getenv('CUSTOM_VAR'), $_ENV['CUSTOM_VAR'] ?? null, $_SERVER['CUSTOM_VAR'] ?? null);
var_dump(isset($_SERVER['HTTP_PROXY']), $_SERVER['HTTP_X_FORWARDED_FOR']);
