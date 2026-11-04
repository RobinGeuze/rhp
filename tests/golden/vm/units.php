<?php
// Every load of a file (the main script, each include, each eval) resolves
// its file-local closure and class indices in its own unit.
$early = function () { return 'main closure'; };
$early2 = fn() => 'main arrow';

include __DIR__ . '/units_lib.inc';
echo $fromLib(), "\n";
echo withDefault(), "\n";
$d = (new ReflectionFunction('withDefault'))->getParameters()[0]->getDefaultValue();
echo $d(), "\n";
$attr = (new ReflectionFunction('tagged'))->getAttributes()[0]->newInstance();
echo ($attr->make)(), "\n";
echo (Consts::MAKE)(), "\n";
declareNested();
$k = (new Consts)->counter();
echo Nested::N, " ", $k(), $k(), "\n";
echo $early(), " ", $early2(), "\n";

// A trait method runs in the trait's unit, whichever file uses it.
class UsesMaker
{
    use Maker;
}
echo ((new UsesMaker)->make())(), " ", UsesMaker::$label, "\n";

// One file, included three times: each include is a unit of its own.
$rows = [];
$kept = [];
for ($round = 1; $round <= 3; $round++) {
    include __DIR__ . '/units_loop.inc';
}
foreach ($rows as $r) {
    echo implode(',', $r), "\n";
}
foreach ($kept as $c) {
    echo $c(), ' ';
}
echo "\n", Round1::R, Round2::R, "\n";

// eval()'d code: a fresh unit (and fresh statics) every time.
for ($i = 0; $i < 3; $i++) {
    echo eval('static $e = 0; return ++$e;');
}
echo "\n";
for ($i = 0; $i < 3; $i++) {
    $c = eval('return function () { static $q = 0; return ++$q; };');
    echo $c(), $c(), ' ';
}
echo "\n";
