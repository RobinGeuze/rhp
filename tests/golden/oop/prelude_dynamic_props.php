<?php
// Built-in classes whose properties go through a prelude __set: a dynamic
// property raises php's deprecation once, by the object's class flags.
set_error_handler(function ($no, $str) { echo "E$no: $str\n"; return true; });

#[AllowDynamicProperties] class AllowZip extends ZipArchive {}
class PlainZip extends ZipArchive {}
#[AllowDynamicProperties] class AllowPeriod extends DatePeriod {}

$start = new DateTime('2020-01-01');
$step = new DateInterval('P1D');
$objects = [
    new ZipArchive,
    new PlainZip,
    new AllowZip,
    new DatePeriod($start, $step, 2),
    new AllowPeriod($start, $step, 2),
    new XMLReader,
    new DOMDocument,
];
foreach ($objects as $o) {
    echo get_class($o), "\n";
    $o->extra = 1;
    $o->extra = 2;
    var_dump($o->extra, isset($o->extra));
    unset($o->extra);
    var_dump(isset($o->extra));
}
