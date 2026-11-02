<?php
trigger_error("user <b>bold</b> & \"q\"", E_USER_WARNING);
trigger_error("deprecated <i>", E_USER_DEPRECATED);
$r = file_get_contents("/nonexistent/<dir>&file");
var_dump(strlen(...[]) ?? null);
