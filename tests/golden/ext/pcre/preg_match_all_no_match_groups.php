<?php
// PREG_PATTERN_ORDER emits every group (named ones too) even with 0 matches.
var_dump(preg_match_all('/(a)(?<n>b)?/', 'xyz', $m), $m);
var_dump(preg_match_all('/(a)(?<n>b)?/', 'xaz', $m), $m);
var_dump(preg_match_all('/x/', 'abc', $m), $m);
var_dump(preg_match_all('/(?<first>\d)(\d)?/', '', $m, PREG_OFFSET_CAPTURE), $m);
var_dump(preg_match_all('/(a)/', 'b', $m, PREG_SET_ORDER), $m);
