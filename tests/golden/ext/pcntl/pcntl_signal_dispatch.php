<?php
// pcntl signal delivery to PHP callbacks: async (between instructions) and via
// pcntl_signal_dispatch(). Every wait loops until the handler has run, so the
// output does not depend on when the kernel delivers the signal.
$log = [];
$h = function ($signo, $info) use (&$log) {
    $log[] = [$signo, is_array($info), array_keys($info), $info['signo'] === $signo,
              $info['pid'] === posix_getpid()];
};
$thrower = function ($signo) { throw new RuntimeException("handler for " . ($signo === SIGUSR2 ? "USR2" : $signo)); };
function wait_for(array &$log, int $n, bool $dispatch = false): void {
    for ($i = 0; count($log) < $n && $i < 5000; $i++) {
        if ($dispatch) {
            pcntl_signal_dispatch();
        }
        usleep(1000);
    }
}

var_dump(pcntl_signal(SIGUSR1, $h));
var_dump(pcntl_signal_get_handler(SIGUSR1) === $h);

// 1. async delivery
var_dump(pcntl_async_signals(true));
var_dump(posix_kill(posix_getpid(), SIGUSR1));
wait_for($log, 1);
var_dump(count($log), $log[0][0] === SIGUSR1, $log[0][1], $log[0][2], $log[0][3], $log[0][4]);

// 2. synchronous delivery: queued until pcntl_signal_dispatch()
var_dump(pcntl_async_signals(false));
var_dump(posix_kill(posix_getpid(), SIGUSR1));
wait_for($log, 2, true);
var_dump(count($log));
var_dump(pcntl_signal_dispatch()); // nothing pending

// 3. a throwing handler: the exception surfaces in the interrupted code
var_dump(pcntl_signal(SIGUSR2, $thrower));
pcntl_async_signals(true);
try {
    posix_kill(posix_getpid(), SIGUSR2);
    for ($i = 0; $i < 5000; $i++) {
        usleep(1000);
    }
    echo "not interrupted\n";
} catch (RuntimeException $e) {
    echo "caught: ", $e->getMessage(), "\n";
}

// 4. handlers given as strings / restore defaults
var_dump(pcntl_signal(SIGUSR2, 'strlen'), pcntl_signal_get_handler(SIGUSR2));
var_dump(pcntl_signal(SIGUSR1, SIG_IGN), pcntl_signal_get_handler(SIGUSR1));
var_dump(posix_kill(posix_getpid(), SIGUSR1)); // ignored
var_dump(pcntl_signal(SIGUSR1, SIG_DFL), pcntl_signal(SIGUSR2, SIG_DFL));
var_dump(pcntl_async_signals(false));

// 5. alarm
var_dump(pcntl_alarm(0));
echo "done\n";
