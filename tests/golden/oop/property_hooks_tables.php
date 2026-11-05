<?php
// Property tables of hooked objects: var_dump leaves virtual properties out,
// and get_object_vars / json_encode / var_export read a backed property
// without a get hook silently (uninitialized reads as null).
class A {
    public $u { get => 1; }
    public int $t { get => 2; }
    public int $backed { set => $value * 2; }
    public $nb { set { $this->nb = $value; } }
}
$a = new A;
var_dump($a);
var_dump((array) $a);
print_r($a);
echo "\n";
var_dump(get_object_vars($a));
echo json_encode($a), "\n";
var_export($a);
echo "\n";
$a->backed = 3;
var_dump($a, get_object_vars($a));
