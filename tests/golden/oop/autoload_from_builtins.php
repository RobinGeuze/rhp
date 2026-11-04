<?php
$seen = [];
spl_autoload_register(function ($c) use (&$seen) { $seen[] = $c; if ($c === 'Lazy') { eval('class Lazy { public $p; function m() {} }'); } });
var_dump(method_exists('Lazy', 'm'));
var_dump(property_exists('Nope1', 'p'));
var_dump(method_exists('Nope2', 'm'));
var_dump(is_a('Nope3', 'X', true), is_a('Nope4', 'X'));
var_dump(is_subclass_of('Nope5', 'X'), is_subclass_of('Nope6', 'X', false));
var_dump(class_exists('bad name'), class_exists('ok\\Name'), interface_exists('I1'), enum_exists('E1'), class_exists('NoLoad', false));
echo implode(",", $seen), "\n";
