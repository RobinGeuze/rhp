<?php
// The procedural date_*/timezone_* aliases and utf8_encode/utf8_decode.
var_dump(date_create("garbage"), timezone_open("Nope/X"));
$d = date_create("2020-01-02 03:04:05", timezone_open("UTC"));
var_dump(date_format($d, "c"), date_timestamp_get($d), date_format(date_add($d, new DateInterval("P1D")), "Y-m-d"), date_diff(date_create("2020-01-01"), date_create("2020-03-01"))->days);
var_dump(date_interval_format(new DateInterval("P2D"), "%d days"), get_class(date_create_immutable()), timezone_name_get(date_timezone_get($d)), date_modify($d, "+1 day")->format("d"));
var_dump(date_format(date_sub($d, new DateInterval("PT1H")), "H"), date_offset_get($d), date_format(date_date_set($d, 2001, 2, 3), "Y-m-d"), date_format(date_time_set($d, 4, 5), "H:i:s"));
var_dump(utf8_encode("caf\xe9") === "café", utf8_decode("café") === "caf\xe9");
