<?php ini_set('session.save_path', sys_get_temp_dir()); ini_set('session.use_only_cookies', 0); ini_set('session.use_strict_mode', 0); session_start(); echo session_id(), "\n";
