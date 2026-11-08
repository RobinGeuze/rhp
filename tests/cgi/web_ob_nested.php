<?php
ob_start(function($b, $phase){ return "[".$phase.":".strtoupper($b)."]"; }, 4);
echo "hello world, this is longer than four";
echo "more";
ob_start();
echo "inner";
header('X-After: 1');
var_dump(ob_get_level());
$s = ob_get_status(true);
ob_end_flush();
echo "tail";
