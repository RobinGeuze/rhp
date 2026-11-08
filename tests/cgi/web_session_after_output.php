<?php
echo "out";
ini_set('session.save_path', sys_get_temp_dir());
var_dump(session_start());
var_dump(session_status());
