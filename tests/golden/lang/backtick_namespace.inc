<?php
// A backtick calls the global shell_exec(), even in a namespace with its own.
namespace Foo;
function shell_exec($x) { return "mine:$x\n"; }
echo `echo hey`;
echo shell_exec('x');
