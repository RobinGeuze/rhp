<?php
// The id setters and posix_times()/posix_sysconf() (not posix_setsid(): the
// goldens also run in-process).
var_dump(posix_setuid(posix_getuid()), posix_setgid(posix_getgid()));
var_dump(posix_seteuid(posix_geteuid()), posix_setegid(posix_getegid()));
foreach ([[-1, 0], [0, -1]] as [$p, $g]) {
    try { posix_setpgid($p, $g); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
}
var_dump(array_keys(posix_times()), is_int(posix_sysconf(1)));
