<?php
for ($i = 0; $i < 3; $i++) {
    for ($j = 0; $j < 3; $j++) {
        if ($i + $j == 3) break 2;
        echo "$i$j ";
    }
}
echo "done\n";
