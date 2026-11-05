<?php
// $restart_syscalls is Z_PARAM_BOOL_OR_NULL: null is accepted silently.
var_dump(pcntl_signal(SIGUSR1, SIG_DFL, null));
foreach ([[], new stdClass, STDIN, fn() => 1, "x", 0, 1.5] as $v) {
    try {
        var_dump(pcntl_signal(SIGUSR1, SIG_DFL, $v));
    } catch (TypeError $e) {
        echo $e->getMessage(), "\n";
    }
}
