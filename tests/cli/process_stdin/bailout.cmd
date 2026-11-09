opt -R
opt register_shutdown_function(function () { echo "shutdown\n"; }); echo $argn, "\n"; function f() {}
opt -E
opt echo "end\n";
noscript
stdinfile input.txt
