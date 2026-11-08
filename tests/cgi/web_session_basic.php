<?php
$dir = sys_get_temp_dir() . '/rhp-cgi-sess-' . getmypid();
ini_set('session.save_path', $dir);
@mkdir($dir);
session_id('abcdef0123456789abcdef0123');
session_start();
$_SESSION['n'] = ($_SESSION['n'] ?? 0) + 1;
echo session_id(), " ", session_name(), " ", $_SESSION['n'], "\n";
var_dump(headers_list());
session_write_close();
echo file_get_contents($dir.'/sess_abcdef0123456789abcdef0123'), "\n";
array_map('unlink', glob("$dir/sess_*"));
rmdir($dir);
