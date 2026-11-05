<?php
echo "not printed\n";
class NarrowA { protected function f() {} }
class NarrowB extends NarrowA { private function f() {} }
