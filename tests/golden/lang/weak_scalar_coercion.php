<?php
// Coercive-mode scalar parameter coercion (zend_verify_weak_scalar_type_hint):
// int > float > string > bool preference; int|float picks by numeric string.
function t($fn, $v) { try { var_dump($fn($v)); } catch (TypeError $e) { echo preg_replace('/, called in.*/', '', $e->getMessage()), "\n"; } }
function i(int $x) { return $x; } function f(float $x) { return $x; } function s(string $x) { return $x; } function b(bool $x) { return $x; }
function if_(int|float $x) { return $x; } function is(int|string $x) { return $x; } function fs(float|string $x) { return $x; }
function sb(string|bool $x) { return $x; } function ib(int|bool $x) { return $x; } function nf(?float $x) { return $x; }
function fa(false|array $x) { return $x; } function ia(int|array $x) { return $x; } function tf(true|int $x) { return $x; }
foreach (['i','f','s','b','if_','is','fs','sb','ib','nf','fa','ia','tf'] as $fn) {
  foreach ([" 12", "12 ", "12abc", "1e2", "0x1A", "", " ", "abc", 1.0, 1e30, INF, -0.0, true, false, 7, "9223372036854775808", 9.2233720368547758E18, [1]] as $v) {
    echo "$fn ", var_export($v, true), ": "; t($fn, $v);
  }
}
