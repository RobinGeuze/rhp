<?php
echo "a";
flush();
header("X-Too-Late: 1");
setcookie("late", "1");
var_dump(headers_sent($f, $l), basename($f), $l);
