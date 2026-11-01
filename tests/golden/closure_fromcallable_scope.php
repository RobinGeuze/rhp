<?php
class R { public function go() { $c = Closure::fromCallable([$this, 'cb']); return $c(1, 2); } protected function cb($a, $b) { return "cb $a $b"; }
  public function go2() { return Closure::fromCallable([$this, 'priv']); } private function priv() { return "priv"; } }
echo (new R)->go(), "\n";
$p = (new R)->go2(); echo $p(), "\n";
try { Closure::fromCallable([new R, 'cb']); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { Closure::fromCallable('nofn'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$s = Closure::fromCallable('strtoupper'); echo $s("x"), "\n";
