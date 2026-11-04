<?php
echo date("Y-m-d H:i:s", 1700000000), "\n";
echo date("D, d M Y", 1700000000), "\n";
echo gmdate("H:i:s", 3661), "\n";
echo mktime(12, 30, 0, 6, 15, 2023), "\n";
var_dump(checkdate(2, 29, 2024), checkdate(2, 29, 2023));
echo date("N l", 1700000000), "\n";
