<?php
// One global symbol table shared by {main}, top-level includes, `global` and
// $GLOBALS (php 8.1+: element writes go through, whole reads are copies).
include __DIR__ . '/globals_symbol_table.inc';
var_dump($fromInc, $setInInc, $_composer_bin_dir);

function writer() {
    global $fromInc;
    $fromInc = 'changed';
    $GLOBALS['made'] = [1];
    $GLOBALS['made'][] = 2;
    $GLOBALS['made']['k']['deep'] = true;
    $GLOBALS['counter'] = 1;
    $GLOBALS['counter']++;
    $GLOBALS['counter'] += 10;
    $GLOBALS['str'] = 'a';
    $GLOBALS['str'] .= 'b';
}
writer();
var_dump($fromInc, $made, $counter, $str);

// isset / ?? / empty / unset through $GLOBALS
function probe() {
    var_dump(isset($GLOBALS['made']), isset($GLOBALS['nope']), $GLOBALS['nope'] ?? 'dflt');
    var_dump(empty($GLOBALS['made']), isset($GLOBALS['made']['k']['deep']));
    unset($GLOBALS['made']);
    var_dump(isset($GLOBALS['made']));
}
probe();
var_dump(isset($made));

// a whole-array read is a copy
function copier() {
    $g = $GLOBALS;
    $g['counter'] = -1;
    var_dump($GLOBALS['counter'], is_array($g), isset($g['GLOBALS']));
}
copier();

// references and unset at global scope
$a = 1;
$b = &$a;
unset($a);
$a = 2;
var_dump($b, $GLOBALS['a'], $GLOBALS['b']);
$c = 5;
$d = &$c;
function bump() { global $d; $d++; }
bump();
var_dump($c);
$x = &$GLOBALS['counter'];
$x = 99;
var_dump($counter);

// foreach variables at global scope are globals too
foreach (['p' => 1, 'q' => 2] as $fk => $fv) {}
function lastLoop() { return [$GLOBALS['fk'], $GLOBALS['fv']]; }
var_dump(lastLoop());
$arr = [1, 2];
foreach ($arr as &$ref) {}
function lastRef() { global $ref; return $ref; }
var_dump(lastRef());
unset($ref);

// order of user globals in $GLOBALS follows declaration
$z1 = 'one';
$z2 = 'two';
$keys = array_keys($GLOBALS);
var_dump(array_slice($keys, array_search('z1', $keys), 2));

// reading an undefined global warns
set_error_handler(function ($l, $m) { echo "[$l] $m\n"; return true; });
function undefinedRead() { return $GLOBALS['neverSet']; }
var_dump(undefinedRead());
