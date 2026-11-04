<?php
// zend_compile_const_decl: a const statement naming true, false or null (in
// any case, in any namespace) is a compile error, not a redefinition warning.
ob_start(function ($s) { return str_replace(__FILE__, 'F', $s); });
echo "before\n";
eval("namespace N;\nconst A = 1,\n  nULL = 2;");
echo "not reached\n";
