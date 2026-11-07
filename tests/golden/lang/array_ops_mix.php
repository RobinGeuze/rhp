<?php
// Random array operations (inserts, unsets, pointer moves, shift/pop/
// unshift/splice, copies, sorts, by-ref foreach): a digest of everything.
foreach ([1, 2, 3] as $seed) {
mt_srand($seed);
$out = '';
for ($round = 0; $round < 40; $round++) {
    $a = [];
    $copies = [];
    $n = mt_rand(1, 300);
    for ($i = 0; $i < $n; $i++) {
        $op = mt_rand(0, 15);
        switch ($op) {
            case 0: case 1: case 2: $a[] = $i; break;
            case 3: $a[mt_rand(-5, 40)] = $i; break;
            case 4: $a["k" . mt_rand(0, 50)] = $i; break;
            case 5: unset($a[mt_rand(0, 40)]); break;
            case 6: unset($a["k" . mt_rand(0, 50)]); break;
            case 7: $out .= json_encode([key($a), current($a)]); next($a); break;
            case 8: $out .= json_encode([key($a), prev($a)]); break;
            case 9: if (mt_rand(0, 3) == 0) { $out .= json_encode(array_shift($a)); } else { $out .= json_encode(array_pop($a)); } break;
            case 10: array_unshift($a, 'u' . $i); break;
            case 11: $copies[] = $a; break;
            case 12: end($a); $out .= json_encode(key($a)); break;
            case 13: reset($a); break;
            case 14: if (count($a) > 2) { array_splice($a, mt_rand(0, count($a) - 1), mt_rand(0, 2), ['s' . $i]); } break;
            case 15: $a[mt_rand(0, 400)] = $i; break;
        }
    }
    foreach ($a as $k => $v) { $out .= "$k=$v,"; }
    $b = $a; ksort($b); $out .= json_encode($b);
    $b = $a; asort($b); $out .= json_encode(array_keys($b));
    $out .= json_encode([count($a), array_is_list($a), array_key_first($a), array_key_last($a)]);
    $a[] = 'end'; $out .= json_encode(array_key_last($a));
    foreach ($copies as $c) { $out .= json_encode($c) . count($c); }
    foreach ($a as $k => &$v) { if (is_int($k) && $k % 3 == 0) unset($a[$k]); if (is_int($v)) $v++; } unset($v);
    $out .= json_encode($a) . "\n";
}
echo md5($out), "\n";
}
