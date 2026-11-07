<?php
// foreach by reference and array_walk over arrays changed while they run
// (HashTable iterator positions): a digest per seed.
foreach ([1, 2, 3] as $seed) {
mt_srand($seed);
$out = '';
for ($r = 0; $r < 30; $r++) {
    $a = [];
    for ($i = 0; $i < mt_rand(5, 40); $i++) { if (mt_rand(0, 2)) $a[] = $i; else $a["s$i"] = $i; }
    $n = 0;
    foreach ($a as $k => &$v) {
        $n++;
        if ($n > 60) break;
        switch (mt_rand(0, 6)) {
            case 0: unset($a[array_key_first($a)]); break;
            case 1: $a[] = "x$n"; break;
            case 2: $a["n$n"] = $n; break;
            case 3: array_pop($a); break;
            case 4: if (mt_rand(0, 4) == 0) array_shift($a); break;
            case 5: if (mt_rand(0, 4) == 0) array_unshift($a, "u$n"); break;
            case 6: $v = "w$n"; break;
        }
        $out .= "$k,";
    }
    unset($v);
    $w = 0;
    array_walk($a, function (&$v, $k) use (&$a, &$w) { $w++; if ($w < 40 && mt_rand(0, 3) == 0) { unset($a[$k]); $a[] = $k; } });
    $out .= json_encode($a) . json_encode([key($a), current($a)]) . "\n";
}
echo md5($out), "\n";
}
