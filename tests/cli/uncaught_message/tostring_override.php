<?php
class E extends Exception { public function __toString(): string { return 'custom!'; } }
function g() { throw new E(''); }
g();
