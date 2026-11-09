<?php
// zend_scan_escape_string: \X is a hex escape too, and \" / \` decode only
// inside their own quotes (a heredoc keeps both as written).
$x = 'v';
var_dump(bin2hex("\X1C\x41\X4a\X\x"));
var_dump("q\`\"\u{41}\8\101\\");
var_dump(<<<EOT
  a\"$x\`\X41\$
    ["te\"st"]
  EOT);
var_dump(<<<EOT
a\"b\X41
EOT);
var_dump('a\'\\\X\"');
var_dump(b"\X41");
