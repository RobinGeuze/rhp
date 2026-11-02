<?php
echo "a\n";
$f = fopen('php://stdout', 'w');
fwrite($f, "raw\n");
echo "b\n";
