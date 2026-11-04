<?php
// Included once per round.
$counter = function () { static $n = 0; return ++$n; };
static $top = 0;
$top++;
$rows[] = [$counter(), $counter(), $top];
$kept[] = $counter;
if (!class_exists("Round$round", false)) {
    if ($round === 1) { class Round1 { const R = 1; } }
    if ($round === 2) { class Round2 { const R = 2; } }
}
