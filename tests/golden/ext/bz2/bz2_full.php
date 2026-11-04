<?php
$big = '';
$x = 7;
for ($i = 0; $i < 30000; $i++) {
    $x = ($x * 1103515245 + 12345) & 0x7fffffff;
    $big .= chr(97 + ($x >> 16) % 5);
}
foreach (['', 'abc', str_repeat('hello ', 1000), $big] as $s) {
    foreach ([[4, 0], [1, 0], [9, 0], [9, 30], [5, 250]] as [$b, $w]) {
        $c = bzcompress($s, $b, $w);
        echo strlen($s), " b$b w$w: ", strlen($c), ' ', md5($c), "\n";
        var_dump(bzdecompress($c) === $s, bzdecompress($c, true) === $s);
    }
}
echo bin2hex(bzcompress('')), "\n", bin2hex(bzcompress('abc', 9, 30)), "\n";
var_dump(bzdecompress(''), bzdecompress('abc'), bzdecompress('BZh9xxxxxxxxxxxx'));
var_dump(bzdecompress(substr(bzcompress('hello world'), 0, 20)));
var_dump(bzdecompress(bzcompress('hi') . 'junk'));
foreach ([fn() => bzcompress('a', 0), fn() => bzcompress('a', 10), fn() => bzcompress('a', 4, -1), fn() => bzcompress('a', 4, 251)] as $f) {
    try {
        $f();
    } catch (ValueError $e) {
        echo $e->getMessage(), "\n";
    }
}
$dir = sys_get_temp_dir() . '/rhp_golden_bz2';
@mkdir($dir);
var_dump(file_put_contents("compress.bzip2://$dir/a.bz2", "bzip2 stream data\nline 2\n"));
var_dump(bin2hex(file_get_contents("$dir/a.bz2")));
var_dump(file_get_contents("compress.bzip2://$dir/a.bz2"));
$b = bzopen("$dir/b.bz2", 'w');
var_dump(is_resource($b), bzwrite($b, 'written with bzwrite'), bzclose($b));
var_dump(bzdecompress(file_get_contents("$dir/b.bz2")));
$b = bzopen("$dir/b.bz2", 'r');
var_dump(bzread($b, 7), bzread($b), bzclose($b));
file_put_contents("$dir/c.bz2", bzcompress('first|') . bzcompress('second'));
var_dump(file_get_contents("compress.bzip2://$dir/c.bz2"));
try {
    bzopen("$dir/b.bz2", 'rw');
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}
var_dump(extension_loaded('bz2'));
foreach (['a.bz2', 'b.bz2', 'c.bz2'] as $p) {
    unlink("$dir/$p");
}
rmdir($dir);
