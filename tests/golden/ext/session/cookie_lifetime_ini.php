<?php
// session.cookie_lifetime parses as a quantity (OnUpdateLongGEZero), with
// its warning, through ini_set and session_start's options.
ob_start();
set_error_handler(function ($no, $msg) { echo "[$no] $msg\n"; return true; });
foreach (['abc', '12x', '5k', '-3', '7'] as $v) {
    var_dump(ini_set('session.cookie_lifetime', $v), ini_get('session.cookie_lifetime'));
}
var_dump(session_start(['cookie_lifetime' => 'q', 'save_path' => sys_get_temp_dir()]));
var_dump(ini_get('session.cookie_lifetime'));
session_abort();
