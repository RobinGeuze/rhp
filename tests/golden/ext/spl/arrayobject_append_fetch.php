<?php
// `$ao[][] = v` calls a user offsetGet with no offset: a required or
// variadic $offset is "not passed", a default is used.
set_error_handler(function ($no, $msg, $file, $line) { echo "$msg (line $line)\n"; return true; });
class R extends ArrayObject { function offsetGet($k): mixed { return []; } }
class V extends ArrayObject { function offsetGet(...$k): mixed { return []; } }
class D extends ArrayObject { function offsetGet($k = 'dflt'): mixed { echo "get($k)\n"; return []; } }
foreach ([new R, new V, new D] as $ao) {
    try { $ao[][] = 1; } catch (ArgumentCountError $e) { echo $e->getMessage(), "\n"; }
}
// A property fetch in a constant expression converts its name when it runs.
enum E: string { case A = 'a'; const B = E::A->{[]}; }
echo "start\n";
var_dump(E::B);
