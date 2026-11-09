opt -B
opt echo "begin\n";
opt -R
opt var_dump($argi, $argn);
opt -E
opt echo "end $argi\n";
noscript
stdinfile input.txt
