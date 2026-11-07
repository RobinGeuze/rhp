<?php
// foreach by reference over an ArrayIterator backed by an object reaches
// the object's public property slots (spl_array's object storage).

#[AllowDynamicProperties]
class P {
    public $a = 1;
    protected $b = 2;
    private $c = 3;
    public int $d = 4;
}

$o = new P;
$o->dyn = 5;
$it = @new ArrayIterator($o);
foreach ($it as $k => &$v) {
    echo "$k\n";
    $v = $v * 10;
}
unset($v);
var_dump($o);

// The typed property stays a type source of the reference.
foreach (@new ArrayIterator($o) as $k => &$v) {
    if ($k === 'd') {
        try {
            $v = 'not an int';
        } catch (TypeError $e) {
            echo $e->getMessage(), "\n";
        }
        $v = '7';
    }
}
var_dump($o->d);

// An ArrayIterator over an array is unchanged.
$arr = [1, 2];
$ai = new ArrayIterator($arr);
foreach ($ai as &$v) {
    $v++;
}
unset($v);
var_dump($ai->getArrayCopy(), $arr);
