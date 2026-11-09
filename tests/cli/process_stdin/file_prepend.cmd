ini auto_prepend_file={DIR}/pre.inc
opt -F
opt file.php
opt -B
opt set_exception_handler(function ($e) { echo "handled\n"; });
noscript
stdinfile input.txt
