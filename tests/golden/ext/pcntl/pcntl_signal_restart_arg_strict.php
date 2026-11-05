<?php
declare(strict_types=1);
// $restart_syscalls is Z_PARAM_BOOL_OR_NULL: null is no TypeError.
var_dump(pcntl_signal(SIGUSR1, SIG_DFL, null));
var_dump(pcntl_signal(SIGUSR1, SIG_DFL, false));
