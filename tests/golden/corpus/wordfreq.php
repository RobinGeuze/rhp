<?php
$text = "the cat sat on the mat the cat ran the dog sat";
$words = preg_split('/\s+/', trim($text));
$freq = array_count_values($words);
arsort($freq);
foreach ($freq as $w => $n) {
    echo str_pad($w, 6) . str_repeat('#', $n) . " ($n)\n";
}
// top 2 with stable tie behaviour
$top = array_slice($freq, 0, 2, true);
echo "top: " . implode(", ", array_keys($top)) . "\n";
// group words by length
$byLen = [];
foreach (array_unique($words) as $w) {
    $byLen[strlen($w)][] = $w;
}
ksort($byLen);
foreach ($byLen as $len => $ws) {
    sort($ws);
    echo "$len: " . implode(",", $ws) . "\n";
}
