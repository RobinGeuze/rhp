<?php
// A trait alias naming a magic method enums may not have.
ob_start(fn ($s) => str_replace(__FILE__, "F", $s));
trait T { function bar($n) { return "bar $n"; } }
echo "start\n";
enum E { case A; use T { bar as __get; } }
echo "unreachable\n";
