<?php
// A builtin that runs past max_execution_time + hard_timeout: the
// process dies (exit 124) naming the file and line being executed.
function f()
{
    return password_hash("x", PASSWORD_BCRYPT, ["cost" => 17]);
}
echo "start\n";
f();
