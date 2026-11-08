<?php function f() { throw new LogicException("bad & <worse>"); } try { f(); } finally { echo "fin\n"; }
