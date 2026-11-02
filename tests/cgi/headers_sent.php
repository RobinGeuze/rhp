<?php
var_dump(headers_sent($f, $l), $f, $l);
echo "x\n";
var_dump(headers_sent($f, $l), basename($f), $l);
header("X-Late: 1");
var_dump(http_response_code(404));
var_dump(setcookie("late", "1"));
