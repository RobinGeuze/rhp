<?php
set_error_handler(function ($no, $msg, $file, $line) { echo "[$no] $msg (line $line)\n"; return true; });
$csv = "id,name,score\n1,Alice,95.5\n2,Bob,82\n3,Carol,77.25";
$lines = explode("\n", $csv);
$header = str_getcsv(array_shift($lines));
$records = [];
foreach ($lines as $line) {
    $records[] = array_combine($header, str_getcsv($line));
}
foreach ($records as $r) {
    printf("%-2s %-8s %6.2f\n", $r['id'], $r['name'], (float)$r['score']);
}
$avg = array_sum(array_map(fn($r) => (float)$r['score'], $records)) / count($records);
printf("avg: %.3f\n", $avg);
usort($records, fn($a, $b) => (float)$b['score'] <=> (float)$a['score']);
echo "winner: " . $records[0]['name'] . "\n";
// rebuild csv
$out = implode(",", $header) . "\n";
foreach ($records as $r) $out .= implode(",", array_values($r)) . "\n";
echo $out;
