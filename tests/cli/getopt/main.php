<?php
// getopt() reads the request's argv, not $_SERVER['argv'].
var_dump(getopt('abc::d:', ['long::', 'req:', 'flag', 'no-cache', 'threads:', '1'], $rest), $rest);
$_SERVER['argv'] = ['ignored', '-a'];
var_dump(getopt('a'));
