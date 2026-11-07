<?php
ob_start(fn ($s) => str_replace(__FILE__, 'F', $s));
$s = new SplObjectStorage; $o = new stdClass;
$s->attach($o, 1); var_dump($s->contains($o)); $s->detach($o); var_dump(count($s));
class T extends SplObjectStorage { function offsetSet($k, $v = null): void { echo "os\n"; parent::offsetSet($k, $v); } function offsetExists($k): bool { echo "oe\n"; return parent::offsetExists($k); } function offsetUnset($k): void { echo "ou\n"; parent::offsetUnset($k);} }
$t = new T; $t->attach($o); var_dump($t->contains($o)); $t->detach($o); $t[$o] = 2; var_dump(isset($t[$o])); unset($t[$o]);
$r = new ReflectionMethod('SplObjectStorage', 'attach'); var_dump($r->isDeprecated(), count($r->getAttributes()));
set_error_handler(function($n, $m, $f, $l) { echo "$m @ ", basename($f), ":$l\n"; return true; });
$s->attach($o);
$s->addAll($t); $s->removeAll($t);
class U extends SplObjectStorage { function getHash($o): string { echo "gh\n"; return parent::getHash($o); } }
$u = new U; $o = new stdClass; $u->attach($o); var_dump($u->contains($o)); $u->detach($o);
$s = new SplObjectStorage; try { $s->attach(1); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$m = new ReflectionMethod('SplObjectStorage','contains'); echo $m;
