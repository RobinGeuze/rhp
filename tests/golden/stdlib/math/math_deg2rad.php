<?php
// php computes (deg / 180.0) * M_PI and (rad / M_PI) * 180.
foreach ([1.1, 0.3, 7, 45, 90, 180, 270, 360, -33.3, 1e-300, 1e300, 123.456, 0.1, 2.5, 1000000.7] as $d) {
    var_dump(deg2rad($d));
}
foreach ([1.1, 0.3, 7, M_PI, M_PI / 2, M_PI / 4, -2.2, 1e-300, 1e300, 123.456, 0.1, 2.5, 3.0] as $r) {
    var_dump(rad2deg($r));
}
$diff = 0;
for ($i = 1; $i <= 2000; $i++) {
    $x = $i * 0.37;
    if (rad2deg(deg2rad($x)) !== $x) {
        $diff++;
    }
}
echo $diff, "\n";
