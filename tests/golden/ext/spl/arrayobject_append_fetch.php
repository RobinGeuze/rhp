<?php
// `$ao[][] = v` calls a user offsetGet with no offset: a required or
// variadic $offset is "not passed", a default is used. (The notice that
// follows is masked: it carries the path.)
class R extends ArrayObject { function offsetGet($k): mixed { return []; } }
class V extends ArrayObject { function offsetGet(...$k): mixed { return []; } }
class D extends ArrayObject { function offsetGet($k = 'dflt'): mixed { echo "get($k)\n"; return []; } }
error_reporting(E_ALL & ~E_NOTICE);
foreach ([new R, new V, new D] as $ao) {
    try { $ao[][] = 1; } catch (ArgumentCountError $e) { echo $e->getMessage(), "\n"; }
}
error_reporting(E_ALL);
set_error_handler(function ($no, $msg, $file, $line) { echo "$msg (line $line)\n"; return true; });
// A property fetch in a constant expression converts its name when it runs.
enum E: string { case A = 'a'; const B = E::A->{[]}; }
echo "start\n";
var_dump(E::B);
