<?php
$a = []; $s = 0;
for ($i = 0; $i < 3000000; $i++) { $a[$i & 1023] = $i; $s += $a[($i * 7) & 1023] ?? 0; $s ^= $i << 1; }
$str = ''; foreach ($a as $k => $v) { $str .= $k; }
echo $s, strlen($str), "\n";
