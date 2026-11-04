<?php
// Failing filesystem builtins raise php's warnings (composer's Cache /
// Filesystem rely on the ErrorException and on error_get_last()).
set_error_handler(function ($l, $m, $f, $n) {
    if (str_contains($m, 'include_path')) { $m = preg_replace('/include_path=.*/', 'include_path=...)', $m); }
    echo "[$l] $m\n";
    return true;
});
$x = '/nonexistent/x';
var_dump(fopen($x, 'r'), file_get_contents($x), file($x), file_put_contents($x, 'a'), copy($x, '/nonexistent/c'), readfile($x));
var_dump(mkdir('/nonexistent/a/b'), rmdir($x), unlink($x), rename($x, '/nonexistent/z'));
var_dump(touch('/nonexistent/t'), chmod($x, 0644), filemtime($x), filesize($x), fileperms($x), stat($x), lstat($x), readlink($x), filetype($x));
var_dump(opendir($x), scandir($x), dir($x), file_exists($x), is_file($x), is_dir($x), is_readable($x), is_link($x));
var_dump(fopen('some/file', 'q'), fopen('nope://x', 'r'), fopen('php://nope', 'r'));
try { fopen('', 'r'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(include 'nope_missing.php');
set_error_handler(function ($l, $m, $f, $n) { throw new ErrorException($m, 0, $l, $f, $n); });
try { file_put_contents($x, 'data'); } catch (ErrorException $e) { echo "caught: ", $e->getMessage(), "\n"; }
restore_error_handler(); restore_error_handler();
var_dump(@unlink($x), error_get_last()['message']);
