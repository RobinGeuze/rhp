<?php
// spl_object_storage_compare_objects: storages compare their entries' info
// (unordered), a subclass is uncomparable, properties do not count.
class S extends SplObjectStorage {}
$o = new stdClass; $p = new stdClass;
$a = new SplObjectStorage; $b = new SplObjectStorage;
$a[$o] = 1; $b[$o] = 2; var_dump($a == $b, $a < $b, $a <=> $b);
$b[$o] = 1; var_dump($a == $b);
$b[$p] = 1; var_dump($a == $b, $a <=> $b);
$c = new SplObjectStorage; $c[$p] = 1; $a2 = new SplObjectStorage; $a2[$o] = 1; var_dump($a2 == $c);
$s = new S; $t = new S; var_dump($s == $t, $s == new SplObjectStorage, $a == new stdClass);
@$a->dyn = 1; $d = new SplObjectStorage; $d[$o] = 1; var_dump($a == $d);
