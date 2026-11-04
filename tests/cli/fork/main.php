<?php
echo "before\n";
$p = pcntl_fork();
if ($p === 0) { echo "child\n"; exit(3); }
$w = pcntl_waitpid($p, $s);
var_dump($p > 0, $w === $p);
var_dump(pcntl_wexitstatus($s), pcntl_wifexited($s), pcntl_wifsignaled($s));

// a child killed by a signal
$p = pcntl_fork();
if ($p === 0) { posix_kill(posix_getpid(), SIGKILL); sleep(5); exit(0); }
var_dump(pcntl_wait($s) === $p, pcntl_wifsignaled($s), pcntl_wtermsig($s) === SIGKILL);

// WNOHANG, resource usage
$p = pcntl_fork();
if ($p === 0) { usleep(200000); exit(7); }
var_dump(pcntl_waitpid($p, $s, WNOHANG));
var_dump(pcntl_waitpid($p, $s, 0, $ru) === $p, pcntl_wexitstatus($s), isset($ru['ru_utime.tv_sec']));
var_dump(pcntl_waitpid(-1, $s, WNOHANG), pcntl_get_last_error() === PCNTL_ECHILD);

// output buffers are duplicated into the child
ob_start();
echo "buffered ";
$p = pcntl_fork();
echo $p === 0 ? "in-child\n" : "in-parent\n";
if ($p === 0) { ob_end_flush(); exit(0); }
pcntl_waitpid($p, $s);
ob_end_flush();

// a socket pair between parent and child
[$a, $b] = stream_socket_pair(STREAM_PF_UNIX, STREAM_SOCK_STREAM, 0);
$p = pcntl_fork();
if ($p === 0) { fclose($a); fwrite($b, "from child"); exit(0); }
fclose($b);
var_dump(stream_get_contents($a));
pcntl_waitpid($p, $s);

// shutdown functions run in the child too
$parent = getmypid();
register_shutdown_function(function () use ($parent) { echo "shutdown ", getmypid() === $parent ? "parent" : "child", "\n"; });
$p = pcntl_fork();
if ($p === 0) { exit(0); }
pcntl_waitpid($p, $s);

var_dump(pcntl_sigprocmask(SIG_BLOCK, [SIGUSR1], $old), $old);
var_dump(pcntl_sigprocmask(SIG_SETMASK, [], $old), $old === [SIGUSR1]);
echo "end\n";
