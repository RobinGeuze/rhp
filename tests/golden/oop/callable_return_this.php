<?php
class P { function f(): callable { return [self::class, 'h']; } function g(): callable { return 'self::h'; } private function h() { return 1; }
  function pr(): callable { return [static::class, 'h']; } static function s(): callable { return [self::class, 'h']; } function k(): callable { return 'P::nope'; } }
class Q extends P {}
$p = new P; var_dump(is_callable($p->f()), $p->g());
try { (new Q)->pr(); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { P::s(); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { $p->k(); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
