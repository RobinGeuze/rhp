<?php
// zend_register_constant: a name that is taken (a constant, true / false /
// null in any case, __COMPILER_HALT_OFFSET__) warns and defines nothing,
// for define() and for a const statement alike.
set_error_handler(function ($no, $msg) {
    echo "warning: $msg\n";
    return true;
});
const A = 1;
const A = 2;
define('B', 1);
const B = 3;
var_dump(define('A', 4), define('true', 1), define('NULL', 1), define('False', 1));
var_dump(define('__COMPILER_HALT_OFFSET__', 1));
var_dump(A, B, get_defined_constants(true)['user']);
