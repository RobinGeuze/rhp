<?php
// serialize / unserialize / var_export / print_r / json_encode / (array) of
// objects with shadowed privates, protected and dynamic properties.
class R1 { private $x = 'r1'; protected $shared = 's'; public function x1() { return $this->x; } }
#[AllowDynamicProperties]
class R2 extends R1 { private $x = 'r2'; public $pub = [1, 'a' => null]; public function x2() { return $this->x; } }
$r = new R2;
$r->dyn = 1.5;
$s = serialize($r);
echo str_replace("\0", '~', $s), "\n";
$u = unserialize($s);
echo $u->x1(), ' ', $u->x2(), "\n";
var_dump($u == $r);
print_r($u);
echo "\n";
var_export($u);
echo "\n";
var_export([1 => $u, 'nested' => ['o' => new stdClass, 'e' => []]]);
echo "\n";
$std = new stdClass;
$std->a = "it's";
$std->{'0'} = 'zero';
$std->b = ['x' => "nul\0byte"];
var_export($std);
echo "\n";
echo json_encode($u), "\n";
echo json_encode([$std, $u], JSON_PRETTY_PRINT), "\n";
foreach ((array)$u as $k => $v) echo str_replace("\0", '~', $k), ' => ', json_encode($v), "\n";

class Sleepy extends R1 {
    private $y = 'yy'; protected $z = 'zz'; public $w = 'ww';
    public function __sleep() { return ['y', 'z', 'w', 'shared']; }
    public function __wakeup() { echo "woke ", $this->y, "\n"; }
}
$ss = serialize(new Sleepy);
echo str_replace("\0", '~', $ss), "\n";
print_r(unserialize($ss));
echo "\n";

class Custom {
    private $a = 1; protected $b = 2;
    public function __serialize(): array { return ['a' => $this->a, 'b' => $this->b, 'extra' => [1, 2]]; }
    public function __unserialize(array $data): void { echo "unserialize ", json_encode($data), "\n"; $this->a = $data['a'] * 10; }
}
$cs = serialize(new Custom);
echo $cs, "\n";
print_r(unserialize($cs));
echo "\n";
print_r(unserialize(serialize(['list' => [new R1, new stdClass], 'k' => 'v'])));
echo "\n";
try { serialize(function () {}); } catch (Exception $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
var_export(PHP_INT_MIN);
echo "\n";
var_export(-0.0);
echo "\n";
var_export(1e100);
echo "\n";
