<?php
// get_debug_type() / gettype() for every value kind, incl. anonymous classes.
namespace App\Model;

interface Marker {}
interface Other {}
enum Status { case On; }
class Base {}
class User extends Base {}

$fn = function () {};
$arrow = fn() => 1;
$anon = new class {};
$anonExt = new class extends Base {};
$anonIface = new class implements Marker, Other {};
$anonBoth = new class(3) extends \ArrayObject implements Marker {
    public function __construct(public int $n) { parent::__construct([]); }
};
$res = fopen('php://memory', 'r');
$closed = fopen('php://memory', 'r');
fclose($closed);

$values = [
    null, true, false, 0, -1, 1.5, NAN, "", "s", [], [1],
    $fn, $arrow, \Closure::fromCallable('strlen'),
    new User(), new \stdClass(), new \ArrayObject([]), Status::On,
    $anon, $anonExt, $anonIface, $anonBoth,
    $res, $closed, STDERR,
    new \Exception(), new \TypeError(),
];
foreach ($values as $v) {
    echo get_debug_type($v), " / ", gettype($v), "\n";
}
echo $anonBoth->n, "\n";
var_dump($anonIface instanceof Marker, $anonIface instanceof Other, $anonBoth instanceof \ArrayObject);
echo str_contains(get_class($anon), "class@anonymous") ? "anon-name-ok" : "bad", "\n";
var_dump((new \ReflectionClass($anon))->isAnonymous(), (new \ReflectionClass(new User()))->isAnonymous());
// Two evaluations of the same anonymous class expression share one class.
function mk() { return new class {}; }
var_dump(get_class(mk()) === get_class(mk()));
var_dump(get_class($anon) === get_class($anonExt));
print_r($anon); echo "\n"; print_r($anonExt); echo "\n";
