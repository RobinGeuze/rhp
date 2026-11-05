<?php
declare(strict_types=1);
// In strict mode from()/tryFrom() take exactly the backing type.
enum E: int { case A = 1; }
enum S: string { case A = "1"; }
foreach ([fn() => E::from("1"), fn() => E::tryFrom(1.0), fn() => S::from(1), fn() => S::tryFrom(null)] as $f) {
    try {
        var_dump($f());
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
var_dump(E::from(1), S::tryFrom("1"));
