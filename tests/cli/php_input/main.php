<?php
var_dump(fopen("/dev/null", "r"));
$f = fopen("PHP://INPUT", "r");
var_dump($f, stream_get_meta_data($f)["uri"]);
var_dump(file_get_contents("php://input"));
var_dump(fopen("php://input", "r"));
