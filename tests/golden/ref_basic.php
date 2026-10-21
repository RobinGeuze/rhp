<?php
$a = 1;
$b =& $a;
$b = 5;
echo $a, "\n";
$a = 10;
echo $b, "\n";
$x = "hi";
$y = &$x;
$y .= " there";
echo $x, "\n";
