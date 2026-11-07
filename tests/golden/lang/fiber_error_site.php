<?php
// An error raised before the fiber's function runs any user code is
// reported at the resumer's line (php links the fiber's bottom frame to it).
$f = new Fiber('strlen');
try { $f->start(); } catch (Error $e) { echo get_class($e), " line ", $e->getLine(), ": ", $e->getMessage(), "\n"; }
$f = new Fiber('str_repeat');
try { $f->start("a", -1); } catch (ValueError $e) { echo "line ", $e->getLine(), ": ", $e->getMessage(), "\n"; }
set_error_handler(function ($no, $msg, $file, $line) { echo "line $line: ", preg_replace('/\{closure:.*\}/U', '{closure}', $msg), "\n"; return true; });
$f = new Fiber(function (&$r) { $r = 9; });
$x = 1;
$f->start($x);
var_dump($x);
restore_error_handler();
// The stack limit (a stack below the reserve: hit at once) is raised in
// the frame entered, before its first instruction: its trace shows that
// frame, and no catch in it applies.
ini_set('fiber.stack_size', '32K');
$f = new Fiber(function () { try { echo "never\n"; } catch (Error $e) { echo "never caught\n"; } });
try { $f->start(); } catch (Error $e) {
    echo "line ", $e->getLine(), ", ", count($e->getTrace()), " frames: ", preg_replace('/\d+ bytes/', 'N bytes', $e->getMessage()), "\n";
}
