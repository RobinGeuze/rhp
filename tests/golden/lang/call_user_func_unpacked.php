<?php
declare(strict_types=1);
// php compiles call_user_func[_array]() with plain positional arguments to a
// user call, so the callback sees the caller's strict_types. With an unpacked
// or named argument it is an ordinary internal call: the callback is
// coercive, has no "called in" site, and call_user_func shows in the trace.
function msg(Throwable $e): void {
    $m = $e->getMessage();
    $p = strpos($m, ', called in ');
    echo get_class($e), ': ', $p === false ? $m : substr($m, 0, $p) . ' [site]', "\n";
}
function f(int $x) { return $x; }
function frames(int $x) {
    $names = array_map(
        fn($f) => str_starts_with($f['function'], '{closure') ? '{closure}' : $f['function'],
        debug_backtrace(),
    );
    return implode(' < ', array_slice($names, 0, 2));
}
function inc(int &$x) { $x++; }

$cases = [
    'plain' => fn() => call_user_func('f', "5"),
    'plain array' => fn() => call_user_func_array('f', ["5"]),
    'unpacked' => fn() => call_user_func(...['f', "5"]),
    'unpacked array' => fn() => call_user_func_array(...['f', ["5"]]),
    'unpacked args' => fn() => call_user_func('f', ...["5"]),
    'named forwarded' => fn() => call_user_func('f', x: "5"),
    'unpacked, bad value' => fn() => call_user_func(...['f', "five"]),
    'named, bad value' => fn() => call_user_func('f', x: "five"),
    'unpacked builtin' => fn() => call_user_func(...['strlen', 12345]),
    'trace plain' => fn() => call_user_func('frames', 1),
    'trace unpacked' => fn() => call_user_func(...['frames', 1]),
    'trace unpacked array' => fn() => call_user_func_array(...['frames', [1]]),
];
foreach ($cases as $what => $case) {
    echo "$what: ";
    try {
        var_dump($case());
    } catch (TypeError $e) {
        msg($e);
    }
}

$n = 1;
call_user_func_array(...['inc', [&$n]]);
var_dump($n);
