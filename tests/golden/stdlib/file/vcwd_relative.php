<?php
// Relative paths after chdir(): every file function, include, child
// processes and the C libraries resolve them against the new directory.
set_error_handler(function ($l, $m) { echo "[$l] $m\n"; return true; });
$base = realpath(sys_get_temp_dir()) . "/rhp_vcwd_" . bin2hex(random_bytes(6));
mkdir($base);
mkdir("$base/sub");
file_put_contents("$base/inc.php", '<?php return "included " . basename(__FILE__);');
file_put_contents("$base/sub/data.txt", "one\ntwo\n");
$start = getcwd();

var_dump(chdir($base), getcwd() === $base);
var_dump(chdir("sub"), basename(getcwd()));
var_dump(file_get_contents("data.txt"), file("data.txt", FILE_IGNORE_NEW_LINES));
var_dump(file_exists("data.txt"), is_file("data.txt"), is_dir("../sub"), filesize("data.txt"));
var_dump(realpath("data.txt") === "$base/sub/data.txt", realpath("") === "$base/sub", realpath(".") === "$base/sub");
var_dump(chdir(".."), getcwd() === $base);

// include and require, through include_path's "." and as ./ paths
var_dump(include "inc.php", require "./sub/../inc.php", stream_resolve_include_path("inc.php") === "$base/inc.php");

// writing and file management
var_dump(file_put_contents("w.txt", "abc"), file_put_contents("w.txt", "de", FILE_APPEND), file_get_contents("$base/w.txt"));
$h = fopen("sub/f.txt", "w+");
fwrite($h, "via fopen");
rewind($h);
var_dump(fread($h, 100), stream_get_meta_data($h)["uri"]);
fclose($h);
var_dump(copy("w.txt", "sub/c.txt"), file_get_contents("$base/sub/c.txt"));
var_dump(rename("sub/c.txt", "r.txt"), file_exists("$base/r.txt"), unlink("r.txt"), file_exists("$base/r.txt"));
var_dump(mkdir("a/b/c", 0777, true), is_dir("$base/a/b/c"), rmdir("a/b/c"), rmdir("a/b"), rmdir("a"));
var_dump(touch("t.txt", 1000000000), filemtime("t.txt"), stat("t.txt")["mtime"], lstat("t.txt")["size"]);
var_dump(chmod("t.txt", 0640), decoct(fileperms("t.txt") & 0777), is_readable("t.txt"), is_writable("t.txt"), is_executable("t.txt"));
var_dump(symlink("w.txt", "lnk"), readlink("lnk"), is_link("lnk"), file_get_contents("lnk"), filetype("lnk"), linkinfo("lnk") > 0);
var_dump(link("w.txt", "hard"), fileinode("hard") === fileinode("w.txt"), unlink("hard"), unlink("lnk"));
var_dump(disk_free_space(".") > 0, disk_total_space("sub") > 0);
$t = tempnam("sub", "pre");
var_dump(dirname($t) === "$base/sub", str_starts_with(basename($t), "pre"));
unlink($t);

// directories
$names = scandir("sub");
var_dump($names);
$d = opendir("sub");
$seen = [];
while (($e = readdir($d)) !== false) $seen[] = $e;
closedir($d);
sort($seen);
var_dump($seen === $names, glob("sub/*.txt"), glob("*.php"));
var_dump(iterator_count(new FilesystemIterator("sub")), (new SplFileInfo("sub/data.txt"))->getSize());
$o = new SplFileObject("sub/data.txt");
var_dump($o->fgets(), $o->getRealPath() === "$base/sub/data.txt");
$o = null;
var_dump(dir("sub")->read() !== false);

// child processes start in the working directory
var_dump(trim(shell_exec("pwd -P")) === $base, exec("cat sub/data.txt", $out), $out);
system("ls sub | sort");
passthru("cat w.txt; echo");
$p = popen("pwd -P", "r");
var_dump(trim(fgets($p)) === $base);
pclose($p);
$proc = proc_open(["cat", "data.txt"], [1 => ["pipe", "w"]], $pipes, "sub");
var_dump(stream_get_contents($pipes[1]));
fclose($pipes[1]);
proc_close($proc);
$proc = proc_open("cat w.txt > sub/out.txt", [], $pipes);
proc_close($proc);
$proc = proc_open("pwd -P", [1 => ["file", "sub/pwd.txt", "w"]], $pipes);
proc_close($proc);
var_dump(file_get_contents("sub/out.txt"), trim(file_get_contents("sub/pwd.txt")) === $base);

// chdir failures keep the old directory
var_dump(chdir("missing"), chdir("w.txt"), chdir(""), getcwd() === $base);

// clean up
foreach (["sub/data.txt", "sub/f.txt", "sub/out.txt", "sub/pwd.txt", "w.txt", "t.txt", "inc.php"] as $f) unlink($f);
var_dump(chdir($start), rmdir("$base/sub"), rmdir($base), getcwd() === $start);
