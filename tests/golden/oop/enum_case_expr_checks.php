<?php
// Backed-enum case values from constant expressions are checked against the
// backing type, and duplicate values are rejected, when the enum is loaded.
function t(callable $f) {
    try {
        var_dump($f());
    } catch (Error $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
enum D: int { case A = Y; case B = 2; }
define('Y', 'str');
t(fn() => D::A->value);
t(fn() => D::B);           // any constant fetch loads the whole enum
t(fn() => count(D::cases())); // cases() only evaluates the case values
t(fn() => D::from(2));

enum D2: string { case A = 1 + 1; }
t(fn() => D2::A);

enum E: int { case A = X; case B = 2; const Q = 1; }
define('X', 2);
t(fn() => E::B);
t(fn() => E::Q);
t(fn() => E::from(2));
t(fn() => E::tryFrom(2));

enum F: string { case A = "x"; case B = "y"; case C = "x"; }
t(fn() => F::B);

enum G: int { case A = 1.0; }
t(fn() => G::A);

enum H: string { case A = "a" . "b"; case B = self::A->value . "c"; }
t(fn() => H::B->value);
t(fn() => H::from("abc"));
t(fn() => H::tryFrom("zz"));

enum I: int { case A = 1 << 3; case B = PHP_INT_SIZE; case C = self::A->value + 1; }
t(fn() => array_map(fn($c) => $c->value, I::cases()));
t(fn() => I::from(9));

enum J: string { case A = 'a'; case B = 'b'; }
t(fn() => J::from('b'));
