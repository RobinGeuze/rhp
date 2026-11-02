<?php
ini_set('display_errors', 'stderr');
trigger_error("to stdout under cgi-fcgi", E_USER_NOTICE);
echo "done\n";
