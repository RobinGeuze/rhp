<?php
// php-cli's constants and functions do not exist under php-cgi.
var_dump(defined('STDIN'), defined('STDOUT'), defined('STDERR'));
var_dump(array_key_exists('STDIN', get_defined_constants()));
var_dump(function_exists('cli_set_process_title'), function_exists('cli_get_process_title'));
var_dump(ignore_user_abort(), ignore_user_abort(true), ignore_user_abort());
var_dump(connection_status(), connection_aborted(), CONNECTION_NORMAL);
