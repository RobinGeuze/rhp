<?php
// eval() converts its operand like zval_try_get_string: __toString for an object.
class S { function __toString(): string { return 'echo "evald\n"; return 5;'; } }
var_dump(eval(new S));
try { eval(new stdClass); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
var_dump(eval(42 . ';'));
var_dump(eval('return 7;'));
class T { function __toString(): string { throw new Exception("ts"); } }
try { eval(new T); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
// "" compiles to nothing: eval() is false.
var_dump(eval(''), eval(null), eval(false), eval(' '));
