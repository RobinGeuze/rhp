<?php
// ZEND_END_SILENCE: an error_reporting set to fatal-only inside `@` goes
// back to the value from before the `@`; any other value stays.
@ini_set('error_reporting', '0');
var_dump(error_reporting());
error_reporting(E_ALL);
@(function () { error_reporting(0); error_reporting(E_ALL); error_reporting(0); })();
var_dump(error_reporting());
@(function () { error_reporting(E_ERROR); })();
var_dump(error_reporting());
@error_reporting(E_ALL & ~E_NOTICE);
var_dump(error_reporting());
error_reporting(E_ALL);
@(function () { @(function () { error_reporting(0); })(); var_dump(error_reporting()); })();
var_dump(error_reporting());
function thrower() {
    try {
        @(function () { ini_set('error_reporting', '0'); throw new Exception(); })();
    } catch (Exception $e) {
    }
}
thrower();
var_dump(error_reporting());
error_reporting(0);
@(function () { error_reporting(0); })();
var_dump(error_reporting());
