<?php
// Process introspection, opcache-off API, getopt without CLI options, fnmatch.
$ru = getrusage();
var_dump(array_keys($ru), is_int($ru['ru_utime.tv_sec']), array_keys(getrusage(1)) === array_keys($ru));
$gc = gc_status();
var_dump(array_keys($gc), $gc['runs'], $gc['collected'], $gc['threshold'], $gc['running'], is_float($gc['application_time']));
var_dump(memory_reset_peak_usage(), is_int(gc_mem_caches()), count(sys_getloadavg()));
var_dump(opcache_get_status(), opcache_get_status(false), opcache_invalidate(__FILE__), opcache_is_script_cached(__FILE__), opcache_reset());
var_dump(getopt('ab:c::', ['long', 'req:'], $rest), $rest, getopt(''));

$f = fopen('php://memory', 'r');
var_dump(stream_set_read_buffer($f, 0), stream_set_chunk_size($f, 100), stream_set_chunk_size($f, 50));
try { stream_set_chunk_size($f, 0); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }

// FNM_NOESCAPE / FNM_PATHNAME are the C library's values (they differ by platform).
var_dump(FNM_NOESCAPE !== FNM_PATHNAME, FNM_PERIOD, FNM_CASEFOLD);
$cases = [
    ['*.php', 'a.php', 0], ['*.php', 'dir/a.php', 0], ['*.php', 'dir/a.php', FNM_PATHNAME],
    ['a?c', 'abc', 0], ['a?c', 'ac', 0], ['[a-c]x', 'bx', 0], ['[!a-c]x', 'dx', 0], ['[!a-c]x', 'ax', 0],
    ['*', '.hidden', FNM_PERIOD], ['*', '.hidden', 0], ['\*', '*', 0], ['\*', '\*', FNM_NOESCAPE],
    ['A*', 'abc', FNM_CASEFOLD], ['A*', 'abc', 0], ['src/*/x', 'src/a/x', FNM_PATHNAME], ['', '', 0],
];
$flname = fn($f) => match ($f) {
    0 => '0', FNM_NOESCAPE => 'NOESCAPE', FNM_PATHNAME => 'PATHNAME', FNM_PERIOD => 'PERIOD', FNM_CASEFOLD => 'CASEFOLD',
};
foreach ($cases as [$p, $s, $fl]) {
    echo json_encode($p), ' ', json_encode($s), ' ', $flname($fl), ' => ', var_export(fnmatch($p, $s, $fl), true), "\n";
}
