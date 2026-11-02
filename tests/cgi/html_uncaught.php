<?php
function f() { throw new RuntimeException("bad <thing> & \"stuff\""); }
echo "before\n";
f();
