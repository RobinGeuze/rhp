<?php
// A "pty" descriptor: the child gets the slave end, $pipes the master
// end, opened read-write.
$p = proc_open('read x; echo "got:$x" >&2', [0 => ['pty'], 1 => ['pty'], 2 => ['pipe', 'w']], $pipes);
var_dump(count($pipes));
foreach ($pipes as $k => $s) {
    echo $k, ' ', stream_get_meta_data($s)['mode'], "\n";
}
fwrite($pipes[0], "hello\n");
echo stream_get_contents($pipes[2]);
var_dump(proc_close($p));
