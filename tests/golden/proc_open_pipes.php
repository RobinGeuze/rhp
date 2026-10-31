<?php
// proc_open / proc_get_status / proc_close / proc_terminate and pipe streams.
function t($f) {
    try {
        var_dump($f());
    } catch (\Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
function status($p) {
    $s = proc_get_status($p);
    var_dump(is_int($s['pid']) && $s['pid'] > 0);
    unset($s['pid']);
    var_dump($s);
}

// Three pipes; write stdin, read stdout line by line and stderr in one go.
$p = proc_open("cat; echo err >&2; exit 5", [0 => ["pipe", "r"], 1 => ["pipe", "w"], 2 => ["pipe", "w"]], $pipes);
var_dump($p, $pipes, get_resource_type($p));
var_dump(fwrite($pipes[0], "hello\nworld\nlast"));
fclose($pipes[0]);
var_dump(fgets($pipes[1]), fgets($pipes[1], 3), fgets($pipes[1]));
var_dump(feof($pipes[1]));
var_dump(fgets($pipes[1]), fgets($pipes[1]), feof($pipes[1]));
var_dump(stream_get_contents($pipes[2]));
var_dump(proc_close($p));

// Status of an exited process: cached after the first observation.
$p = proc_open(["sh", "-c", "exit 7"], [], $pipes);
var_dump($pipes);
usleep(200000);
status($p);
status($p);
var_dump(proc_close($p));

// A signaled process: not cached, so the second read reports exitcode -1.
$p = proc_open("exec sleep 5", [], $pipes);
var_dump(proc_terminate($p));
usleep(200000);
status($p);
status($p);
var_dump(proc_close($p));

// cwd + env (empty values dropped, int keys taken verbatim).
$p = proc_open('echo "$FOO-${BAR-unset}-$BAZ"; pwd', [1 => ["pipe", "w"]], $pipes, "/", ["FOO" => "x", "BAR" => "", 5 => "BAZ=1"]);
var_dump(stream_get_contents($pipes[1]));
var_dump(proc_close($p));

// redirect and null descriptors.
$p = proc_open("echo out; echo err >&2", [1 => ["pipe", "w"], 2 => ["redirect", 1]], $pipes);
var_dump(stream_get_contents($pipes[1]));
proc_close($p);
$p = proc_open("echo gone; echo e >&2; exit 1", [1 => ["null"], 2 => ["null"]], $pipes);
var_dump($pipes, proc_close($p));

// A file descriptor target, and a higher fd number.
$f = sys_get_temp_dir() . "/rhp_proc_open_" . getmypid() . ".txt";
$p = proc_open("echo to-file; echo fd3 >&3", [1 => ["file", $f, "w"], 3 => ["pipe", "w"]], $pipes);
var_dump(array_keys($pipes), stream_get_contents($pipes[3]));
proc_close($p);
var_dump(file_get_contents($f));
$p = proc_open("echo appended", [1 => ["file", $f, "a"]], $pipes);
proc_close($p);
var_dump(file_get_contents($f));
unlink($f);

// Array command (no shell): metacharacters are passed literally.
$p = proc_open(["printf", "%s|%s", "a b", '$HOME'], [1 => ["pipe", "w"]], $pipes);
var_dump(stream_get_contents($pipes[1]));
$s = proc_get_status($p);
var_dump($s['command']);
proc_close($p);
t(fn() => @proc_open(["rhp-no-such-command-xyz"], [], $pipes));

// Non-blocking reads and stream_select (keys are preserved).
$p = proc_open("sleep 0.3; echo late", [1 => ["pipe", "w"]], $pipes);
var_dump(stream_set_blocking($pipes[1], false));
var_dump(fread($pipes[1], 100), feof($pipes[1]));
$r = ["k" => $pipes[1]];
$w = null;
$e = null;
var_dump(stream_select($r, $w, $e, 0, 0), $r);
$r = ["k" => $pipes[1]];
var_dump(stream_select($r, $w, $e, 5), array_keys($r));
var_dump(fread($pipes[1], 100));
$r = [3 => $pipes[1]];
var_dump(stream_select($r, $w, $e, 5), array_keys($r));
var_dump(fread($pipes[1], 100), feof($pipes[1]));
proc_close($p);

// The resources are identical to themselves only.
$p = proc_open("true", [1 => ["pipe", "w"], 2 => ["pipe", "w"]], $pipes);
var_dump($pipes[1] === $pipes[1], $pipes[1] === $pipes[2], array_search($pipes[2], $pipes, true));
proc_close($p);

// Argument validation.
t(fn() => proc_open([], [], $pipes));
t(fn() => proc_open([""], [], $pipes));
t(fn() => proc_open(["a", "b\0"], [], $pipes));
t(fn() => proc_open("true", ["a" => 1], $pipes));
t(fn() => proc_open("true", [1], $pipes));
t(fn() => @proc_open("true", [["foo"]], $pipes));
t(fn() => proc_open("true", [[]], $pipes));
t(fn() => proc_open("true", [["pipe"]], $pipes));
t(fn() => proc_open("true", [["file", "x"]], $pipes));
t(fn() => proc_open("true", [["file"]], $pipes));
t(fn() => proc_open("true", [["redirect", "1"]], $pipes));
t(fn() => proc_open("true", [["redirect"]], $pipes));
t(fn() => @proc_open("true", [["redirect", 7]], $pipes));
t(fn() => @proc_open("true", [fopen("php://memory", "r")], $pipes));
t(fn() => proc_close(fopen("php://memory", "r")));
t(fn() => proc_get_status(fopen("php://memory", "r")));
t(fn() => proc_close("nope"));
$r = [];
t(fn() => stream_select($r, $w, $e, 0));
$r = [STDIN];
t(fn() => stream_select($r, $w, $e, -1));
t(fn() => stream_select($r, $w, $e, 0, -1));
t(fn() => stream_select($r, $w, $e, null, 5));

// isset on string offsets (used by Symfony Process' read loop).
$s = "";
var_dump(isset($s[0]), isset($s[-1]));
$s = "ab";
var_dump(isset($s[1]), isset($s[2]), isset($s[-2]), isset($s[-3]), isset($s["1"]), isset($s["x"]), isset($s[0][0]), isset($s[1][1]));
$a = ["x" => null, "y" => [1]];
var_dump(isset($a["x"]), isset($a["y"][0]), isset($a["y"][1]), isset($a["z"]["q"]));
