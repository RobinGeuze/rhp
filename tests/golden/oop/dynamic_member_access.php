<?php
// Property and method names from variables and expressions.
class Box {
    public $a = 1;
    function greet($n) { return "hi $n"; }
}
$o = new Box();
$p = 'a'; echo $o->$p, "\n";
$o->$p = 10; echo $o->a, "\n";
$o->{'a'} = 20; echo $o->{'a'}, "\n";
$m = 'greet'; echo $o->$m('x'), "\n";
