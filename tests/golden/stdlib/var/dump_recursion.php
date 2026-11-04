<?php
// Cycles: var_dump / print_r print *RECURSION*, var_export prints NULL (php
// also warns, silenced here).
$o = new stdClass; $o->name = 'o'; $o->self = $o; $o->list = [$o];
var_dump($o);
print_r($o);
echo "\n";
@var_export($o);
echo "\n";
$a = [1]; $a[] = &$a;
var_dump($a);
print_r($a);
echo "\n";
@var_export($a);
echo "\n";
// The same (non-cyclic) object twice is not recursion.
$shared = new stdClass; $shared->v = 1;
var_dump([$shared, $shared]);
var_export([$shared, $shared]);
echo "\n";
print_r([$shared, $shared]);
// Deep but finite nesting.
$n = new stdClass; $n->child = new stdClass; $n->child->child = new stdClass;
print_r($n);
echo "\n";
