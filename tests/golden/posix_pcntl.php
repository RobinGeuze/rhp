<?php
// ext/posix + ext/pcntl surface (values compared structurally: they depend on
// the machine, so only their shape and mutual consistency are printed).
function t($f) {
    try {
        var_dump($f());
    } catch (\Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}

var_dump(extension_loaded('posix'), extension_loaded('pcntl'));
var_dump(posix_getpid() === getmypid(), is_int(posix_getppid()));
var_dump(posix_getuid() >= 0, posix_geteuid() >= 0, posix_getgid() >= 0, posix_getegid() >= 0);

$pw = posix_getpwuid(posix_geteuid());
var_dump(array_keys($pw), $pw['uid'] === posix_geteuid(), is_string($pw['name']));
var_dump(posix_getpwnam($pw['name'])['uid'] === $pw['uid']);
var_dump(posix_getpwnam("rhp-no-such-user-xyz"));
$gr = posix_getgrgid(posix_getegid());
var_dump(array_keys($gr), $gr['gid'] === posix_getegid(), is_array($gr['members']));
$u = posix_uname();
var_dump($u['sysname'] === PHP_OS);

var_dump(posix_kill(posix_getpid(), 0));
var_dump(posix_kill(999999999, 0), posix_get_last_error() === 3, posix_strerror(3));

// isatty on a pipe and on a non-descriptor.
$p = proc_open("true", [1 => ["pipe", "w"]], $pipes);
var_dump(posix_isatty($pipes[1]), stream_isatty($pipes[1]));
proc_close($p);
var_dump(@posix_isatty(fopen("php://memory", "r")));

// pcntl signal table.
var_dump(SIG_DFL, SIG_IGN, SIGHUP, SIGINT, SIGQUIT, SIGKILL, SIGTERM, WNOHANG);
var_dump(pcntl_async_signals(), pcntl_async_signals(true), pcntl_async_signals(), pcntl_async_signals(null));
var_dump(pcntl_signal_get_handler(SIGINT));
$h = function ($sig) { echo "got $sig\n"; };
var_dump(pcntl_signal(SIGINT, $h));
var_dump(pcntl_signal_get_handler(SIGINT) === $h);
var_dump(pcntl_signal(SIGUSR1, SIG_IGN), pcntl_signal_get_handler(SIGUSR1));
var_dump(posix_kill(posix_getpid(), SIGUSR1)); // ignored: we survive
var_dump(pcntl_signal(SIGUSR1, SIG_DFL), pcntl_signal_get_handler(SIGUSR1));
var_dump(pcntl_signal(SIGINT, SIG_DFL));
var_dump(pcntl_signal(SIGTERM, 'strlen'), pcntl_signal_get_handler(SIGTERM));
var_dump(pcntl_signal(SIGTERM, SIG_DFL));
var_dump(pcntl_signal_dispatch());
t(fn() => pcntl_signal(0, SIG_DFL));
t(fn() => pcntl_signal(SIGINT, 5));
t(fn() => pcntl_signal(SIGINT, "no_such_function_xyz"));
// The upper bound is the platform's SIGRTMAX / NSIG-1: print it masked.
try { pcntl_signal_get_handler(0); } catch (ValueError $e) { echo preg_replace('/\d+$/', 'N', $e->getMessage()), "\n"; }
var_dump(pcntl_wifexited(0), pcntl_wexitstatus(3 << 8), pcntl_wifsignaled(9), pcntl_wtermsig(9));
