<?php
// Coercive typing converts an object with __toString to string for a
// `string` parameter / return type (unless the type also accepts the object).
class S { function __toString(): string { return "s!"; } }
function r(): string { return new S; }
function a(string $x) { return $x; }
function n(?string $x) { return $x; }
function u(int|string $x) { return $x; }
function k(S|string $x) { return $x; }
var_dump(r(), a(new S), n(new S), u(new S), k(new S) instanceof S);
var_dump(array_map(fn(string $s) => strlen($s), [new S]));
try { (function (int $i) {})(new S); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
