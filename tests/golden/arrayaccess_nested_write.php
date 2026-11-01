<?php
// Writes through a nested lvalue that reaches an ArrayAccess object: the
// leaf is offsetSet / offsetUnset, a deeper path goes through offsetGet
// (php-cs-fixer's `$this->tokens[$i] = new Token(...)`).
set_error_handler(function ($l, $m) { echo "[$l] $m\n"; return true; });

class A implements ArrayAccess
{
    public $d = [];
    public function offsetExists($o): bool { echo "exists $o\n"; return isset($this->d[$o]); }
    public function offsetGet($o): mixed { echo 'get ', var_export($o, true), "\n"; return $this->d[$o] ?? null; }
    public function offsetSet($o, $v): void
    {
        echo 'set ', var_export($o, true), "\n";
        if ($o === null) { $this->d[] = $v; } else { $this->d[$o] = $v; }
    }
    public function offsetUnset($o): void { echo "unset $o\n"; unset($this->d[$o]); }
}

$o = new stdClass;
$o->t = new A;
$o->t[2] = 'b';

$arr = ['k' => new A];
$arr['k'][3] = 'c';
unset($arr['k'][4]);
$arr['k'][5] .= 'x';
$arr['k'][] = 'app';
$arr['k'][7] = 1;
$arr['k'][7]++;              // RW fetch: offsetGet only, the increment is lost
var_dump($arr['k']->d);

$a = new A;
$a[1] = new A;
$a[1][2] = 'nested';         // offsetGet(1) returns an object: no notice
var_dump($a[1]->d);
$a[7] = 'q';
$a[7] .= 'r';
var_dump($a->d[7]);
$a['s']['t'] = 1;            // offsetGet('s') is not an object: notice, no effect

class H
{
    public $t;
    public function __construct() { $this->t = new A; }
    public function go()
    {
        $this->t[9] = 'h';
        $this->t[] = 'z';
        unset($this->t[9]);
        return $this->t->d;
    }
}
var_dump((new H)->go());

var_dump(isset($arr['k'][3]), $arr['k'][3] ?? 'd', empty($arr['k'][99]));

// non-ArrayAccess containers
foreach ([
    function () { $i = 5; $i[0] = 1; },
    function () { $x = ['a' => 5]; $x['a']['b'] = 1; },
    function () { $s = new stdClass; $s[1] = 2; },
    function () { $s = ['o' => new stdClass]; $s['o']['k'] = 2; },
    function () { $t = true; $t[] = 1; },
] as $f) {
    try {
        $f();
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}
