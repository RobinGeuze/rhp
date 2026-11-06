<?php
// A namespaced constant named like a deprecated built-in one is not
// deprecated; the built-in one still is.
namespace Foo;
set_error_handler(function ($no, $str) {
    echo "[$no] $str\n";
    return true;
});
const FILE_TEXT = 1;
echo FILE_TEXT, \Foo\FILE_TEXT, constant('Foo\FILE_TEXT'), "\n";
echo \FILE_TEXT, "\n";
echo E_STRICT, "\n";
