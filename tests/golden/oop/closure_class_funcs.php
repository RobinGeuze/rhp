<?php
$f = function () {};
var_dump(get_class($f), get_parent_class($f), get_object_vars($f));
class A {
    public $prop { get { return function () { return $this->prop; }; } }
}
$c = (new A)->prop;
var_dump(get_class($c), get_class($c()));
