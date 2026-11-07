<?php
// `$o->$m($x[0])` on a temporary receiver: asking whether the parameter is by
// reference must leave the receiver for the call.
class H { function call(...$a) { echo "ok ", count($a), "\n"; return $this; } }
class J { public $h; function mk() { return new H; }
  function a() { $this->h = new H; $m = 'call'; $p = [1]; $this->h->{$m}($p[0]); }
  function b() { $m = 'call'; $p = [1]; $this->mk()->{$m}($p[0]); }
  function c() { $m = 'call'; $p = [1]; (new H)->$m($this->h); }
  function d() { $m = 'call'; $p = [1]; $this->mk()->$m($p); }
  function e() { $p = [1]; $this->mk()->{'ca' . 'll'}($p[0])->{'call'}($p[0], $p[0]); }
}
$j = new J; foreach (['a', 'b', 'c', 'd', 'e'] as $f) { try { echo "$f: "; $j->$f(); } catch (\Throwable $e) { echo $e->getMessage(), "\n"; } }
