<?php
ob_start(fn ($s) => str_replace(__FILE__, 'F', $s));
class P { function f(): callable { return [self::class, 'h']; } function g(): callable { return 'self::h'; } private function h() { return 1; }
  function pr(): callable { return [static::class, 'h']; } static function s(): callable { return [self::class, 'h']; } function k(): callable { return 'P::nope'; } }
class Q extends P {}
$p = new P; var_dump(is_callable($p->f()), $p->g());
try { (new Q)->pr(); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { P::s(); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { $p->k(); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
// A string or array the union takes as such is not looked up as a callable.
class U { private function h() {} function r(): callable|string { return 'self::h'; } function r2(): array|callable { return ['self', 'h']; }
  function a(callable|string $x) { return $x; } function go() { var_dump($this->r(), $this->r2(), $this->a('self::h')); } }
(new U)->go();
