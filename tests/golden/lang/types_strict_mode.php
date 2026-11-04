<?php
// declare(strict_types=1): parameters and returns take only their exact
// type, except int widening to float.
declare(strict_types=1);
function f(int $x): int { return $x; }
function g(float $y): float { return $y; }
echo f(5), "\n";
echo g(3), "\n";
try { f("5"); } catch (\TypeError $e) { echo str_replace(__DIR__ . '/', '', $e->getMessage()), "\n"; }
try { f(true); } catch (\TypeError $e) { echo "TE-bool\n"; }
try { (function(): int { return "x"; })(); } catch (\TypeError $e) { echo "TE-ret\n"; }
