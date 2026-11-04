<?php
set_error_handler(function ($l, $m) { echo "[$l] $m\n"; return true; });
$t = tmpfile();
var_dump(is_resource($t), get_resource_type($t));
fputcsv($t, ['a', 'b c', 'd"e', "multi\nline", '', 'x,y', 'back\\slash']);
fputcsv($t, [1, 2.5, true, null], ';', "'", '\\', "\r\n");
fwrite($t, "\n");
fwrite($t, "plain,row\n");
rewind($t);
echo stream_get_contents($t);
rewind($t);
while (($r = fgetcsv($t)) !== false) { var_dump($r); }
rewind($t);
var_dump(stream_get_line($t, 100, "\n"), stream_get_line($t, 3), stream_get_line($t, 0, "line"));
$m = fopen('php://memory', 'w+'); fwrite($m, "a|b|c"); rewind($m);
var_dump(stream_get_line($m, 10, "|"), stream_get_line($m, 10, "|"), stream_get_line($m, 10, "|"), stream_get_line($m, 10, "|"));
$p = popen('echo hello; echo world', 'r'); var_dump(fgets($p), stream_get_contents($p), pclose($p));
$p = popen('cat > /dev/null; exit 3', 'w'); fwrite($p, "x"); var_dump(pclose($p));
