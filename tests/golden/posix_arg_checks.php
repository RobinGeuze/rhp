<?php
// Argument validation of posix/pcntl/process functions (zpp int coercion,
// php's PID range check): nothing here may reach kill() with a bad pid.
function t($f) {
    try {
        var_dump($f());
    } catch (\Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}

foreach ([4294967295, 4294967296, -2147483649, 2147483648, "abc", "12abc", [], new stdClass, 1e30, NAN, INF] as $pid) {
    t(fn() => posix_kill($pid, 0));
}
// accepted coercions: numeric strings (with whitespace), integral floats, bools
t(fn() => posix_kill((string) posix_getpid(), 0));
t(fn() => posix_kill(" " . posix_getpid() . " ", 0));
t(fn() => posix_kill((float) posix_getpid(), 0));
t(fn() => posix_kill(posix_getpid(), "0"));
t(fn() => posix_kill(posix_getpid(), "x"));
t(fn() => posix_kill(posix_getpid(), 1.5e40));

t(fn() => posix_getpwuid('abc'));
t(fn() => posix_getpwuid([]));
t(fn() => posix_getpwuid(new ArrayObject()));
t(fn() => posix_getpwnam([]));
t(fn() => posix_getgrgid('x'));
t(fn() => posix_getgrnam(new stdClass));
t(fn() => posix_getsid(-1));
t(fn() => posix_getsid(2147483648));
t(fn() => posix_getsid('x'));
t(fn() => posix_getpgid('x'));
t(fn() => posix_getpgid(posix_getpid()) === posix_getpgrp());
t(fn() => posix_strerror('x'));
t(fn() => posix_strerror("2"));
t(fn() => posix_access("a\0b"));
t(fn() => posix_access('/', 'x'));
t(fn() => posix_access('/', POSIX_R_OK));
t(fn() => @posix_isatty('abc'));
t(fn() => @posix_isatty(-1));
t(fn() => posix_get_last_error() === 9);
t(fn() => @posix_ttyname(-1));

t(fn() => pcntl_signal('x', SIG_DFL));
t(fn() => pcntl_signal_get_handler([]));
t(fn() => pcntl_wifexited('x'));
t(fn() => pcntl_wexitstatus("768"));
t(fn() => pcntl_strerror('x'));
t(fn() => pcntl_alarm('x'));
t(fn() => sleep('x'));
t(fn() => sleep(-1));
t(fn() => usleep([]));
t(fn() => usleep(0));
t(fn() => proc_nice('x'));

$p = proc_open('sleep 5', [], $pipes);
t(fn() => proc_terminate($p, 'x'));
t(fn() => proc_terminate(1, 'x'));
t(fn() => proc_terminate($p, 9));
t(fn() => pcntl_wtermsig(proc_close($p) << 8) >= 0);
