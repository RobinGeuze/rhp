<?php
// Shortest round-trip digits where two candidates are equally close: php
// (zend_dtoa mode 0) keeps the even last digit.
$ties = [
    -887777373534812.25,
    123456789012345.625,
    70368744177664.125,
    2 ** -25,
    1152921504606846976.0 + 256,
    9007199254740993.0,
    5e-324,
    1.7976931348623157e308,
];
foreach ($ties as $f) {
    var_dump($f);
    echo json_encode($f), "\n";
    var_export($f);
    echo "\n", serialize($f), "\n";
}
echo json_encode($ties), "\n";
var_export(['k' => 70368744177664.125, [2 ** -25]]);
echo "\n";
