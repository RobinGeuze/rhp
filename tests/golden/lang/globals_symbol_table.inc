<?php
// Included from the global scope: its variables are the globals.
var_dump(is_array($argv), is_int($argc));
$fromInc = 'inc';
function incFn() {
    global $fromInc;
    var_dump($fromInc, $GLOBALS['fromInc'], array_key_exists('argv', $GLOBALS));
}
incFn();
$GLOBALS['setInInc'] = 5;
var_dump($setInInc);
// Composer's bin proxies pass state this way.
$GLOBALS['_composer_bin_dir'] = 'bin';
