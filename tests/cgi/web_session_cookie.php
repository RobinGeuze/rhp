<?php
$dir = sys_get_temp_dir() . '/rhp-cgi-sess-' . getmypid();
ini_set('session.save_path', $dir);
@mkdir($dir);
file_put_contents($dir.'/sess_cookieid1234567890abcdef', 'n|i:41;');
session_start();
$_SESSION['n']++;
echo session_id(), " ", $_SESSION['n'], "\n";
session_regenerate_id(true);
echo strlen(session_id()), "\n";
var_dump(file_exists($dir.'/sess_cookieid1234567890abcdef'));
array_map('unlink', glob("$dir/sess_*"));
rmdir($dir);
