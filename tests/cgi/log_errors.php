<?php
ini_set("display_errors", "0");
ini_set("log_errors", "1");
error_log("custom log line");
$a = [];
echo $a["k"];
trigger_error("user notice");
echo "after\n";
