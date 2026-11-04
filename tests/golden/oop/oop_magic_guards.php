<?php
// Magic accessors are guarded per object + property: inside __get('x') a read
// of $this->x is a plain property access, and so on for __set/__isset/__unset.
#[AllowDynamicProperties]
class Lazy {
    private $loaded = [];
    public function __get($n) {
        echo "__get($n)\n";
        $this->$n = strtoupper($n);   // re-entrant write: plain dynamic property
        return $this->$n;              // re-entrant read: no second __get
    }
    public function __set($n, $v) {
        echo "__set($n)\n";
        $this->$n = "[$v]";            // re-entrant write inside __set
    }
    public function __isset($n) {
        echo "__isset($n)\n";
        return isset($this->$n);
    }
    public function __unset($n) {
        echo "__unset($n)\n";
        unset($this->$n);
    }
}
$l = new Lazy;
echo $l->foo, "\n";
echo $l->foo, "\n";       // now a real property: no __get
$l->bar = 'v';
echo $l->bar, "\n";
var_dump(isset($l->baz), isset($l->bar));
unset($l->bar);
unset($l->qux);
var_dump(isset($l->bar));
print_r($l);
echo "\n";

// A private the caller cannot see goes through __get; the guard is per name.
class Proxy {
    private $data = ['a' => 1];
    private $secret = 's';
    public function __get($n) {
        echo "proxy get $n\n";
        return $n === 'secret' ? $this->secret : ($this->data[$n] ?? null);
    }
}
$p = new Proxy;
var_dump($p->secret, $p->a, $p->missing);
