<?php
// _php_stream_fopen opens expand_filepath()'s result (tsrm_realpath_r):
// a symlink is followed before the `..` after it, `..` after a missing
// directory folds, and a file followed by `/`, `/.` or `/..` fails
set_error_handler(function ($l, $m) { echo "[$l] ", str_replace(__DIR__, 'DIR', $m), "\n"; return true; });
chdir(__DIR__);
$b = 'fopen_expand_filepath.dir';
mkdir("$b/real/sub", 0777, true);
file_put_contents("$b/real/f.txt", "REAL");
file_put_contents("$b/f.txt", "TOP");
symlink(__DIR__ . "/$b/real/sub", "$b/link");
foreach (["link/../f.txt", "nodir/../f.txt", "f.txt/", "f.txt/.", "f.txt/../f.txt", "real/./f.txt", "real//f.txt", "f.txt/x", "link/f.txt"] as $p) {
    $h = fopen("$b/$p", 'r');
    echo $p, ': ', $h ? fread($h, 10) : 'false', "\n";
}
unlink("$b/link");
unlink("$b/real/f.txt");
unlink("$b/f.txt");
rmdir("$b/real/sub");
rmdir("$b/real");
rmdir($b);
