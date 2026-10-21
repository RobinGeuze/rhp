<?php
$a = null;
echo ($a ?? "default"), "\n";
$b = "value";
echo ($b ?? "default"), "\n";
$c = null;
$c ??= "assigned";
echo $c, "\n";
$d = "keep";
$d ??= "ignored";
echo $d, "\n";
