<?php
var_dump(filter_var("AA:BB:CC:DD:EE:FF", FILTER_VALIDATE_MAC));
var_dump(filter_var("aa:bb:cc:dd:ee:ff", FILTER_VALIDATE_MAC));
var_dump(filter_var("AA-BB-CC-DD-EE-FF", FILTER_VALIDATE_MAC));
var_dump(filter_var("AABB.CCDD.EEFF", FILTER_VALIDATE_MAC));
var_dump(filter_var("1A2B.3C4D.5E6F", FILTER_VALIDATE_MAC));
var_dump(filter_var("00:00:00:00:00:00", FILTER_VALIDATE_MAC));
var_dump(filter_var("AA:BB:CC:DD:EE:GG", FILTER_VALIDATE_MAC));
var_dump(filter_var("AA:BB-CC:DD:EE:FF", FILTER_VALIDATE_MAC));
var_dump(filter_var("AA:BB:CC:DD:EE", FILTER_VALIDATE_MAC));
var_dump(filter_var("AABBCCDDEEFF", FILTER_VALIDATE_MAC));
var_dump(filter_var("AA.BB.CC.DD.EE.FF", FILTER_VALIDATE_MAC));
var_dump(filter_var("AABB-CCDD-EEFF", FILTER_VALIDATE_MAC));
