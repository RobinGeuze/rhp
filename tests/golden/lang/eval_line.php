<?php
// An eval/include opcode is at its operand's line (a heredoc's body line).
function where() { $t = debug_backtrace(); return $t[1]['line'] ?? 0; }
echo eval(
    "return where();"
), "\n";
echo eval(<<<'EOT'
return where();
EOT), "\n";
echo eval($c = "return where();"
), "\n";
