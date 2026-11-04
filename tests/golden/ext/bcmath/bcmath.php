<?php
echo bcadd("1.555", "2.333", 2), "\n";
echo bcmul("2.5", "2.5", 4), "\n";
echo bcdiv("10", "3", 6), "\n";
echo bcmod("-10", "3"), "\n";
echo bcpow("2", "10"), "\n";
echo bcpow("2", "-3", 5), "\n";
echo bcsqrt("152.2756", 4), "\n";
echo bccomp("1.00001", "1", 3), bccomp("1.00001", "1", 6), "\n";
echo implode(",", bcdivmod("17", "5", 2)), "\n";
echo bcfloor("-4.3"), " ", bcceil("-4.7"), " ", bcround("2.5"), " ", bcround("12345", -2), "\n";
echo bcpowmod("4", "13", "497"), "\n";
$old = bcscale(3);
echo $old, " ", bcscale(), " ", bcadd("1.11111", "2.22222"), "\n";
echo bcpow("2", "64"), "\n";
