<?php ini_set('session.save_path', sys_get_temp_dir()); session_start(); echo strlen(session_id()), "\n";
