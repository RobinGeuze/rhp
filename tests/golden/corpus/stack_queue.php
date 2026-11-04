<?php
// Shunting-yard-ish: evaluate RPN
function evalRpn(string $expr): float {
    $stack = [];
    foreach (explode(" ", $expr) as $tok) {
        if (is_numeric($tok)) {
            $stack[] = (float)$tok;
        } else {
            $b = array_pop($stack);
            $a = array_pop($stack);
            $stack[] = match ($tok) {
                '+' => $a + $b,
                '-' => $a - $b,
                '*' => $a * $b,
                '/' => $a / $b,
            };
        }
    }
    return $stack[0];
}
echo evalRpn("3 4 + 5 *"), "\n";
echo evalRpn("10 2 / 3 -"), "\n";

// balanced parens
function balanced(string $s): bool {
    $stack = [];
    $pairs = [')' => '(', ']' => '[', '}' => '{'];
    foreach (str_split($s) as $ch) {
        if (in_array($ch, ['(', '[', '{'])) {
            $stack[] = $ch;
        } elseif (isset($pairs[$ch])) {
            if (array_pop($stack) !== $pairs[$ch]) return false;
        }
    }
    return empty($stack);
}
var_dump(balanced("([]{})"));
var_dump(balanced("([)]"));
var_dump(balanced("((("));
