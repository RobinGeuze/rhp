<?php
class K1 { public $z = 'k1'; private $p = 'priv'; function mk() { return function () { return $this->z . $this->p; }; } }
$k = new K1;
$get = function () { return $this->z; };
$b = Closure::bind($get, $k, K1::class);
echo $b(), "\n";
$c = $get->bindTo($k, K1::class);
echo $c(), "\n";
$m = $k->mk();
echo $m(), "\n";
$n = Closure::bind($m, new K1, K1::class);
echo $n(), "\n";
$s = static function () { return K1::class; };
echo $s(), "\n";
echo (fn() => $this->p)->call($k), "\n";
