<?php
// exec / system / passthru / shell_exec / escapeshell* (ext/standard/exec.c).
function t($f) {
    try {
        var_dump($f());
    } catch (\Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}

// exec: output appended to an existing array, trailing whitespace stripped,
// return value is the last line, result code by reference.
$out = ['keep'];
$r = exec("printf 'a  \\nb\\n\\nc \\t\\n'", $out, $code);
var_dump($r, $out, $code);
var_dump(exec("exit 3", $o2, $c2), $o2, $c2);
var_dump(exec("echo hi"));
var_dump(exec("printf 'no newline'"));
var_dump(exec("printf 'x\\n\\n'"));
$notarray = "str";
exec("echo one; echo two", $notarray);
var_dump($notarray);

// system: echoes every line, returns the last one (stripped).
$r = system("echo one; echo two  ", $c);
var_dump($r, $c);
var_dump(system("exit 4", $c), $c);

// passthru: raw output, returns null.
var_dump(passthru("printf 'x\\ny'", $c), $c);
echo "\n";
var_dump(passthru("exit 2", $c), $c);

// shell_exec: whole output, null when empty.
var_dump(shell_exec("printf ''"), shell_exec("echo yo"), shell_exec("printf 'a\\nb'"));

// The environment set with putenv() reaches child processes.
putenv("RHP_PROC_TEST=from-putenv");
var_dump(getenv("RHP_PROC_TEST"), shell_exec('echo $RHP_PROC_TEST'));
putenv("RHP_PROC_TEST");
var_dump(getenv("RHP_PROC_TEST"), shell_exec('echo "[$RHP_PROC_TEST]"'));

// escaping
var_dump(escapeshellarg("it's"), escapeshellarg(""), escapeshellarg("a b\"c"));
var_dump(escapeshellarg("a\xffb\xc3\xa9\xc3x"));
var_dump(escapeshellcmd("a;b 'c' \"d"), escapeshellcmd("x'y"), escapeshellcmd(""));
var_dump(escapeshellcmd("#&;`|*?~<>^()[]{}\$\\\n\xff"));
var_dump(shell_exec("echo " . escapeshellarg("it's \$HOME")));

// argument validation
foreach (['exec', 'system', 'passthru', 'shell_exec', 'escapeshellarg', 'escapeshellcmd'] as $f) {
    t(fn() => $f("a\0b"));
}
foreach (['exec', 'system', 'passthru', 'shell_exec'] as $f) {
    t(fn() => $f(""));
}
t(fn() => putenv("=x"));
t(fn() => putenv(""));
