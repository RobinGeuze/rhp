<?php
printf("%05.2f|%+d|%x|%o|%b|%e\n", 3.14159, 42, 255, 8, 5, 12345.678);
printf("%'*10s|%-10s|%010d\n", "hi", "left", 7);
printf("%1\$s %2\$s %1\$s\n", "a", "b");
echo sprintf("%.3f", 2/3), "\n";
echo number_format(1234567.891, 2), "\n";
echo number_format(1234567.891, 2, ',', '.'), "\n";
echo number_format(0.5), "\n";
echo number_format(-1234.5678, 3, '.', ' '), "\n";
printf("%c%c%c\n", 72, 105, 33);
printf("[%8.3f]\n", -0.0);
printf("%g %G\n", 0.00001234, 123456789.0);
