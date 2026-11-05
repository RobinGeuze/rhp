<?php
// Short-circuit evaluation, coalescing and control flow.

// short_circuit_and_or: the right operand runs only when needed
function t() { echo 'T'; return true; }
function f() { echo 'F'; return false; }
var_dump(f() && t());
var_dump(t() || t());

// logical_not_and_bool_echo
echo !true, '|', !false, "\n";
echo true, '|', false, "\n";
var_dump(1 && 0, 1 || 0, 1 xor 1);

// null_coalesce: a missing nested key
$a = ['x' => null];
echo $a['x'] ?? 'd1', ' ', $a['y'] ?? 'd2', ' ', $a['y']['z'] ?? 'd3', "\n";

// do_while_runs_body_once
$j = 5;
do { echo $j; } while ($j < 3);
echo "\n";

// break_n_and_continue_n
for ($i = 0; $i < 3; $i++) { for ($j = 0; $j < 3; $j++) { if ($i + $j == 2) break 2; echo "$i$j "; } }
echo "\n";
for ($i = 0; $i < 3; $i++) { for ($j = 0; $j < 3; $j++) { if ($j == 1) continue 2; echo "$i$j "; } }
echo "\n";

// switch_fallthrough_and_default
function sw($x) {
    switch ($x) {
        case 1: echo 'one';
        case 2: echo 'two'; break;
        case 3: echo 'three';
        default: echo 'def';
    }
    echo "\n";
}
sw(1); sw(3); sw(9);

// switch_default_before_cases
function sd($x) { switch ($x) { default: return 'd'; case 1: return 'one'; } }
echo sd(1), sd(5), "\n";

// match_is_strict: '1' does not match 1
echo match ('1') { 1 => 'int', default => 'str' }, "\n";

// match_unhandled_throws
try {
    echo match (5) { 1 => 'a', 2 => 'b' };
} catch (Throwable $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
