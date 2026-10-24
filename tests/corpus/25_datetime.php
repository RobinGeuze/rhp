<?php
$ts = mktime(14, 30, 0, 3, 15, 2024);
echo date("Y-m-d H:i:s", $ts), "\n";
echo date("D, d M Y", $ts), "\n";
echo date("N l", $ts), "\n";
echo date("jS \o\f F", $ts), "\n";
$parsed = strtotime("2024-01-01 00:00:00");
$diff = $ts - $parsed;
echo "days: " . intdiv($diff, 86400), "\n";
echo date("L", mktime(0, 0, 0, 1, 1, 2024)), "\n";
echo date("t", mktime(0, 0, 0, 2, 1, 2024)), "\n";
echo date("z", mktime(0, 0, 0, 12, 31, 2023)), "\n";
printf("%02d:%02d\n", intdiv(3661, 3600), intdiv(3661 % 3600, 60));
