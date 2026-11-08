<?php
$t = 0;
for ($i = 0; $i < 20000; $i++) { $t += include __DIR__ . '/inc/a.php'; }
for ($i = 0; $i < 20000; $i++) { include __DIR__ . '/inc/f.php'; $t += $x; }
echo $t, "\n";
