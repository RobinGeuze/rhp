<?php
// CurlHandle / CurlMultiHandle / CurlShareHandle use zend_objects_not_comparable:
// a handle is == only to itself.
$a = curl_init('http://a.invalid/');
$b = curl_init('http://b.invalid/');
$c = curl_init();
$a2 = $a;
echo "== easy\n";
var_dump($a == $b, $a != $b, $a == $a2, $a === $a2, $a == $c);
var_dump($a < $b, $a > $b, $a <= $b, $a >= $b, $a <=> $b, $b <=> $a, $a <=> $a2);
echo "== arrays\n";
var_dump(in_array($c, [$a, $b]), in_array($b, [$a, $b]), array_search($b, [$a, $b]), array_search($c, [$a, $b]));
var_dump(in_array($a, [$a2], true), array_keys([$a, $b, $a], $a));
var_dump([$a] == [$b], [$a] == [$a2]);
echo "== scalars\n";
var_dump($a == 1, 1 == $a, $a == 0, $a == '1', $a == 'x', $a <=> 1, 1 <=> $a);
var_dump($a == null, null == $a, $a <=> null, null <=> $a);
var_dump($a == true, $a == false, (bool) $a);
echo "== multi / share\n";
$m1 = curl_multi_init();
$m2 = curl_multi_init();
$s1 = curl_share_init();
$s2 = curl_share_init();
var_dump($m1 == $m2, $m1 == $m1, $s1 == $s2, $s1 == $s1, $a == $m1, $m1 == $s1, $m1 <=> $m2);
var_dump(in_array($m2, [$m1]), array_search($s2, [$s1, $s2]));
echo "== other objects\n";
$o = new stdClass;
var_dump($a == $o, $o == $a, $a <=> $o, $o <=> $a);
