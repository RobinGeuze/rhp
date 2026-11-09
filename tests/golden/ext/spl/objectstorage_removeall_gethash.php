<?php
// removeAll() stays on an element until its detach fails (getHash twice);
// removeAllExcept() detaches with this storage's own hash.
class S extends SplObjectStorage { public $n = 0; function getHash($o): string { $this->n++; return spl_object_hash($o); } }
$s = new S; $o1 = new stdClass; $o2 = new stdClass; $s[$o1] = 1; $s[$o2] = 2;
$t = new SplObjectStorage; $t[$o1] = 1;
$s->n = 0; var_dump($s->removeAll($t), $s->n);
$s->n = 0; var_dump($s->removeAllExcept($t), $s->n);
$s->n = 0; var_dump($s->addAll($t), $s->n);
$s->n = 0; var_dump($s->removeAll($s), $s->n);
