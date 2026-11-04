<?php
// Coercive mode: scalar parameters and returns convert, arrays and
// non-numeric strings throw.
function f(int $x) { return $x; }
function s(string $x) { return $x; }
function r(): int { return "42"; }
echo f("5"), "\n";
echo f(5.0), "\n";
echo s(42), "\n";
echo var_export(f(true), true), "\n";
echo r(), "\n";
try { f([1]); } catch (\TypeError $e) { echo "TE-arr\n"; }
try { f("abc"); } catch (\TypeError $e) { echo "TE-nan\n"; }
