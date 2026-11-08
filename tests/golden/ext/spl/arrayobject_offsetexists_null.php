<?php
ob_start(fn ($s) => str_replace(__FILE__, 'F', $s));
foreach ([new ArrayObject(['f'=>null, ''=>null, 1=>null, 'x'=>1]), new ArrayIterator(['f'=>null, ''=>null, 1=>null, 'x'=>1])] as $o) {
  var_dump($o->offsetExists('f'), $o->offsetExists('g'), $o->offsetExists(null), $o->offsetExists(1.7), $o->offsetExists(true), $o->offsetExists('x'));
  var_dump(isset($o['f']), isset($o['x']), empty($o['f']), array_key_exists('f', (array)$o->getArrayCopy()));
  try { var_dump($o->offsetExists([])); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}
class S extends ArrayObject { function offsetExists($k): bool { echo "oe\n"; return parent::offsetExists($k); } }
$s = new S(['f'=>null]); var_dump(isset($s['f']), $s->offsetExists('f'));
