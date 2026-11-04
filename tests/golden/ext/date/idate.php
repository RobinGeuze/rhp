<?php
date_default_timezone_set('America/New_York');
foreach ([1710054000, 0, -86399, 1704067199, 4102444800, 1735603200] as $ts) {
  $r = [];
  foreach (str_split('djNwzWmntLyYoBghHGisIZU') as $c) $r[] = "$c=" . idate($c, $ts);
  echo implode(' ', $r), "\n";
}
var_dump(@idate('xx', 0), @idate('Q', 0), @idate('', 0), idate('Y', '86400'));
try { idate('Y', 'abc'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
date_default_timezone_set('Asia/Kolkata'); echo idate('H', 0), idate('i', 0), ' ', idate('Z', 0), ' ', idate('B', 0), "\n";
