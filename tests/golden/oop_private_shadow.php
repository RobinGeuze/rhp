<?php
// Private property shadowing across inheritance: each class's methods see
// their own private slot.
class Base {
    private $name = 'base';
    private $list = [];
    protected $prot = 'p';
    public $pub = 'u';
    public function baseGet() { return $this->name; }
    public function baseSet($v) { $this->name = $v; }
    public function basePush($v) { $this->list[] = $v; $this->list['k'] = $v; return count($this->list); }
    public function baseList() { return $this->list; }
    public function baseIsset() { return isset($this->name); }
    public function baseUnset() { unset($this->name); }
    public function baseRef() { $r = &$this->name; $r = 'via-ref'; }
    public function baseIncr() { $this->name .= '!'; }
    public function baseVars() { return get_object_vars($this); }
    public function baseIter() { $o = []; foreach ($this as $k => $v) { $o[] = $k; } return implode(',', $o); }
    public function baseIterRef() { foreach ($this as $k => &$v) { if (is_string($v)) $v = strtoupper($v); } }
}
#[AllowDynamicProperties]
class Child extends Base {
    private $name = 'child';
    public function childGet() { return $this->name; }
    public function childSet($v) { $this->name = $v; }
    public function childIsset() { return isset($this->name); }
    public function childVars() { return get_object_vars($this); }
    public function childIter() { $o = []; foreach ($this as $k => $v) { $o[] = $k; } return implode(',', $o); }
    public function childList() { return isset($this->list) ? 'set' : 'unset'; }
    public function childSetList() { $this->list = 'dyn'; }
}

$c = new Child;
echo $c->baseGet(), ' ', $c->childGet(), "\n";
$c->baseSet('B1');
echo $c->baseGet(), ' ', $c->childGet(), "\n";
$c->childSet('C1');
echo $c->baseGet(), ' ', $c->childGet(), "\n";
echo $c->basePush('x'), "\n";
var_dump($c->baseList());
var_dump($c->baseIsset(), $c->childIsset());
$c->baseRef();
echo $c->baseGet(), ' ', $c->childGet(), "\n";
$c->baseIncr();
echo $c->baseGet(), "\n";
var_dump($c);
print_r($c); echo "\n";
var_export($c); echo "\n";
echo json_encode($c), "\n";
var_dump(array_keys((array)$c) === ["\0Base\0name", "\0Base\0list", "\0*\0prot", "pub", "\0Child\0name"]);
foreach ((array)$c as $k => $v) echo str_replace("\0", '\0', $k), "\n";
var_dump($c->baseVars(), $c->childVars(), get_object_vars($c));
echo $c->baseIter(), "\n", $c->childIter(), "\n";
foreach ($c as $k => $v) echo "global sees $k\n";
// child sees the parent's private as absent: writes create a dynamic prop
echo $c->childList(), "\n";
$c->childSetList();
echo $c->childList(), "\n";
var_dump($c->baseList());
var_dump(get_object_vars($c));
$c->baseUnset();
var_dump($c->baseIsset(), $c->childIsset());
echo $c->childGet(), "\n";
$c->baseSet('back');
echo $c->baseGet(), "\n";
$c->baseIterRef();
var_dump($c);
// clone keeps both slots independent
$d = clone $c;
$d->baseSet('cloned');
echo $c->baseGet(), ' ', $d->baseGet(), ' ', $d->childGet(), "\n";
