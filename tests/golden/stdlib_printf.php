<?php
printf("%05d|%s|%+d\n", 42, "hi", 9);
vprintf("%s/%s/%d\n", ["a", "b", 7]);
echo vsprintf("%2\$s-%1\$s\n", ["one", "two"]);
fprintf(STDOUT, "[%08.3f]\n", 3.14159);
vfprintf(STDOUT, "%x %o %b\n", [255, 8, 5]);
echo sprintf("%'*10s\n", "pad");
echo sprintf("%-10s|\n", "left");
