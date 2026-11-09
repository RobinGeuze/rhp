<?php
// The POSIX_SC_* and POSIX_PC_* constants in the stub's order, after the
// RLIMIT ones (whose set and values are the platform's).
$names = array_keys(get_defined_constants(true)['posix']);
echo implode("\n", array_filter($names, fn($n) => !str_starts_with($n, 'POSIX_RLIMIT_'))), "\n";
var_dump(array_search('POSIX_RLIMIT_INFINITY', $names) + 1 === array_search('POSIX_SC_ARG_MAX', $names));
var_dump(defined('POSIX_RLIMIT_NOFILE'), posix_sysconf(POSIX_SC_PAGESIZE) > 0, posix_sysconf(POSIX_SC_NPROCESSORS_ONLN) > 0);
