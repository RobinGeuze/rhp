<?php
// Output buffering with user handlers (main/output.c): the $phase bits a
// handler sees, chunked buffers, what a handler's return value does, nested
// buffers, ob_get_status() / ob_list_handlers() shapes and names, ability
// flags, output produced inside a handler, exceptions thrown by handlers.
set_error_handler(function ($no, $msg) {
    echo "[E$no] $msg\n";
    return true;
});
function show($label, $v) { echo $label, ": ", var_export($v, true), "\n"; }
function portable($name) { return preg_replace('/\{closure:.*:(\d+)\}/', '{closure:LINE $1}', $name); }

// phases seen by a handler across flush / clean / end
$log = [];
ob_start(function ($buf, $phase) use (&$log) { $log[] = [$buf, $phase]; return strtoupper($buf); });
echo "one ";
ob_flush();
echo "two ";
ob_clean();
echo "three ";
ob_end_flush();
echo "\n";
show("phases", $log);

// chunked buffer: the handler runs whenever chunk_size bytes are buffered
$log = [];
ob_start(function ($buf, $phase) use (&$log) { $log[] = [$buf, $phase]; return "[$buf]"; }, 3);
echo "ab"; echo "cd"; echo "e"; echo "fghij";
ob_end_flush();
echo "\n";
show("chunks", $log);
$log = [];
ob_start(function ($buf, $phase) use (&$log) { $log[] = [$buf, $phase]; return ""; }, 4);
echo "12345678";
ob_end_clean();
show("chunks discarded", $log);

// return values: false passes the buffer through, true / "" / null eat it
foreach ([true, false, null, "", 0, 42, 1.5, "s"] as $rv) {
    ob_start(function ($b) use ($rv) { return $rv; });
    echo "<data>";
    ob_end_flush();
    echo "| ", var_export($rv, true), "\n";
}
class T { function __toString(): string { return "TS"; } }
ob_start(fn($b) => new T);
echo "x";
ob_end_flush();
echo "\n";
// a handler that returned false is disabled: later output passes through
$calls = 0;
ob_start(function ($b) use (&$calls) { $calls++; return false; });
echo "a";
ob_flush();
echo "b";
ob_flush();
$st = ob_get_status();
ob_end_flush();
echo "\ncalls=$calls flags=", $st['flags'], "\n";

// nested buffers and status
ob_start();
ob_start(fn($b) => $b, 10);
echo "xyz";
$st = ob_get_status(true);
$top = ob_get_status();
$len = ob_get_length();
$lvl = ob_get_level();
ob_end_flush();
$inner = ob_get_clean();
show("status", array_map(fn($s) => ['name' => portable($s['name'])] + $s, $st));
show("top", portable($top['name']) . " level=" . $top['level']);
show("len", $len);
show("level", $lvl);
show("inner", $inner);
show("empty status", ob_get_status());
show("empty full status", ob_get_status(true));
show("no buffer", [ob_get_contents(), ob_get_length(), ob_get_level(), ob_list_handlers(), ob_get_clean()]);

// handler names
class H {
    static function s($b) { return $b; }
    function m($b) { return $b; }
    private function p($b) { return $b; }
    function start() { ob_start([$this, 'p']); }
    function __invoke($b) { return $b; }
}
ob_start('H::s');
ob_start([new H, 'm']);
ob_start(['H', 's']);
ob_start(new H);
(new H)->start();
ob_start(trim(...));
ob_start(fn($b) => $b);
ob_start(null);
show("handlers", array_map('portable', ob_list_handlers()));
for ($i = 0; $i < 8; $i++) ob_end_clean();

// ability flags
foreach ([PHP_OUTPUT_HANDLER_REMOVABLE, PHP_OUTPUT_HANDLER_REMOVABLE | PHP_OUTPUT_HANDLER_CLEANABLE,
          PHP_OUTPUT_HANDLER_STDFLAGS | PHP_OUTPUT_HANDLER_STARTED | PHP_OUTPUT_HANDLER_DISABLED | 0xf,
          PHP_OUTPUT_HANDLER_FLUSHABLE] as $f) {
    ob_start(null, 0, $f);
    echo "data";
    $r = [ob_get_status()['flags'], ob_flush(), ob_clean(), ob_get_length()];
    $e = ob_end_clean();
    if (!$e) {
        // not removable: it stays for the rest of the request, so drain it
        $r[] = ob_get_contents();
    }
    show("flags $f", [$r, $e, ob_get_level()]);
    if (!$e) break;
}
echo "\n";
