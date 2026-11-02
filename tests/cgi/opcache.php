<?php
var_dump(function_exists('opcache_invalidate'), function_exists('opcache_get_status'));
var_dump(opcache_get_status(), opcache_get_status(false));
var_dump(opcache_invalidate(__FILE__), opcache_invalidate(__FILE__, true), opcache_reset());
var_dump(opcache_is_script_cached(__FILE__));
var_dump(opcache_compile_file(__FILE__));
var_dump(opcache_get_configuration());
