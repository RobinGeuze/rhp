<?php
// php://output goes through the output buffers; php://stdout bypasses them.
echo "1\n";
$o = fopen('php://output', 'w');
ob_start(); echo "2-ob\n"; fwrite($o, "3-output-in-ob\n"); $c = ob_get_clean();
echo "captured: ", $c;
// A process resource freed on its last reference closes its pipes.
$p = proc_open('true', [1 => ['pipe', 'w']], $pp); $keep = $pp[1]; $p = null; $pp = null;
var_dump(get_resource_type($keep));
for ($i = 0; $i < 50; $i++) { $p = proc_open('true', [1 => ['pipe', 'w']], $pp); $p = null; $pp = null; }
echo "done\n";
