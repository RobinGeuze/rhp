<?php
$text = "Contact: alice@example.com, bob@test.org; phone 555-1234, 555-5678";
preg_match_all('/([a-z]+)@([a-z]+\.[a-z]+)/', $text, $m);
print_r($m[0]);
echo "users: " . implode(",", $m[1]) . "\n";
echo preg_replace('/\d{3}-\d{4}/', 'XXX-XXXX', $text), "\n";
$csv = "a,b,,c,d";
print_r(preg_split('/,/', $csv));
$out = preg_replace_callback('/\d+/', fn($m) => $m[0] * 2, "1 and 2 and 30");
echo $out, "\n";
echo preg_match('/^\w+$/', "hello_123"), "\n";
echo preg_quote("a.b*c"), "\n";
$date = "2024-03-15";
if (preg_match('/(?<y>\d{4})-(?<m>\d{2})-(?<d>\d{2})/', $date, $mm)) {
    echo "{$mm['y']}/{$mm['m']}/{$mm['d']}\n";
}
echo preg_replace('/(\w+)\s(\w+)/', '$2 $1', "hello world"), "\n";
