<?php
declare(strict_types=1);
// from()/tryFrom() check their argument count; a pure enum has neither;
// called back from an internal function they parse in coercive mode.
enum P { case A; }
enum S: string { case A = '1'; }
$calls = [
    fn() => P::from(1),
    fn() => call_user_func('P::tryFrom', 1),
    fn() => S::from(),
    fn() => S::tryFrom('1', 2),
    fn() => S::cases(1),
    fn() => S::from(1),
    fn() => array_map(S::tryFrom(...), [1, 2]),
];
foreach ($calls as $f) {
    try {
        var_dump($f());
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
