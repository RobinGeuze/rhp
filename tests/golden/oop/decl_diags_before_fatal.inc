<?php
class _ {}
class P { final private function f() {} }
class Q { private function __get($n) {} }
class R { function __isset(int $name) {} }
echo "not printed\n";
