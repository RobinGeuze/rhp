<?php
// get_declared_classes() around a require (phpunit's TestSuiteLoader finds
// the test class that way), get_included_files() order, and a class's
// getFileName().
$before = get_declared_classes();
require_once __DIR__ . '/include_declared_order_b.inc';
include __DIR__ . '/include_declared_order_a.inc';
require_once __DIR__ . '/include_declared_order_c.inc';
var_dump(array_values(array_diff(get_declared_classes(), $before)));
var_dump(array_map('basename', get_included_files()) === array_map('basename', get_required_files()));
foreach (get_included_files() as $f) { echo basename($f), "\n"; }
$t = get_declared_traits(); $i = get_declared_interfaces();
var_dump(trait_exists('FromC'), end($t), end($i));
echo (new ReflectionClass('FromA'))->getFileName() === __DIR__ . '/include_declared_order_a.inc' ? "ok\n" : "bad\n";
