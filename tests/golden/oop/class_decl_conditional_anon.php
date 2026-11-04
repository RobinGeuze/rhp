<?php
// A conditional class declared in a file that also has anonymous classes:
// the nested declaration must not land on an anonymous class's slot.
function make() { return new class { public $x = 1; }; }

if (true) {
    class Bar { public $y = 2; }
}

var_dump(class_exists('Bar', false));
var_dump(make() instanceof Bar);
var_dump((new Bar)->y);

function later() {
    class Later extends Bar {}
}
later();
var_dump(get_parent_class(new Later));
$o = new class extends Bar {};
var_dump($o instanceof Bar, make()->x);
