<?php
var_dump(preg_match('/(?<year>\d{4})-(?<month>\d{2})/', '2024-07', $m));
echo $m['year'], "/", $m['month'], " (", $m[0], ")\n";

preg_match('/\d+/', 'abc123def', $off, PREG_OFFSET_CAPTURE);
echo $off[0][0], " at ", $off[0][1], "\n";

echo preg_match_all('/(\d)(\w)/', '1a2b3c', $all), "\n";
echo implode(",", $all[0]), " | ", implode(",", $all[1]), " | ", implode(",", $all[2]), "\n";

echo preg_replace('/(\w+)\s(\w+)/', '$2 $1', 'hello world'), "\n";
echo preg_replace_callback('/\d+/', fn($m) => $m[0] * 2, 'a3b4c5'), "\n";

print_r(preg_split('/[\s,]+/', "a, b,c  d"));
print_r(preg_split('/([,;])/', 'a,b;c', -1, PREG_SPLIT_DELIM_CAPTURE));

echo preg_quote('1+1=2 (really?)', '/'), "\n";
print_r(preg_grep('/^\d+$/', ['123', 'abc', '456', 'x9']));

var_dump(preg_match('/^[a-z]+$/', 'HELLO'));
var_dump(preg_match('/^[a-z]+$/i', 'HELLO'));

$n = 0;
$r = preg_replace('/a/', 'X', 'banana', -1, $n);
echo "$r count=$n\n";
