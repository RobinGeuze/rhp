<?php
// Backed enum from()/tryFrom() parse their argument before loading the
// class: int for an int-backed enum, int|string (int turned into a
// string) for a string-backed one.
set_error_handler(function ($no, $msg) {
    echo "handler: $no: $msg\n";
    return true;
});
enum E: int { case A = 1; case B = 2; }
var_dump(E::from(true), E::tryFrom(2.0), E::tryFrom("2"), E::tryFrom(" 2"));
foreach ([1.5, null, [], "x", 3] as $v) {
    try {
        var_dump(E::from($v));
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
enum S: string { case A = "1"; case B = "1.5"; }
var_dump(S::from(1), S::tryFrom(1.0), S::tryFrom(true), S::tryFrom(1.5), S::tryFrom(2));
foreach ([new stdClass, [], null, 7] as $v) {
    try {
        var_dump(S::from($v));
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
