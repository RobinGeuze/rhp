<?php
// php_fgetcsv corners: enclosures across lines, escapes, leading space
function show($v) { echo json_encode($v), "\n"; }
show(str_getcsv('a,"b ""c""",d', ',', '"', '\\'));
show(str_getcsv('  "x" ,y', ',', '"', '\\'));
show(str_getcsv("\t\"x\"z,y", ',', '"', '\\'));
show(str_getcsv('"a\\"b",c', ',', '"', '\\'));
show(str_getcsv('"a\\"b",c', ',', '"', ''));
show(str_getcsv('"unterminated,x', ',', '"', '\\'));
show(str_getcsv('"ends with quote"', ',', '"', '\\'));
show(str_getcsv("a,b\r\n", ',', '"', '\\'));
show(str_getcsv("a,\"b\r\n", ',', '"', '\\'));
show(str_getcsv('', ',', '"', '\\'));
show(str_getcsv("\n", ',', '"', '\\'));
show(str_getcsv('a;b|c', ';', '|', '\\'));
foreach ([[',,', '"'], ['', '"'], [',', '""'], [',', '"', 'ab']] as $a) {
    try {
        str_getcsv('x', ...$a);
    } catch (ValueError $e) {
        echo $e->getMessage(), "\n";
    }
}
$f = tmpfile();
fwrite($f, "1,\"multi\nline\r\nfield\",3\n\"open\nnever closed\n");
rewind($f);
while (($r = fgetcsv($f, null, ',', '"', '\\')) !== false) { show($r); }
rewind($f);
show(fgetcsv($f, 4, ',', '"', '\\'));
show(fgetcsv($f, 0, ',', '"', '\\'));
try { fgetcsv($f, -1, ',', '"', '\\'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { fgetcsv($f, 0, 'ab', '"', '\\'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { fputcsv($f, ['a'], ',', '', '\\'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
fclose($f);
