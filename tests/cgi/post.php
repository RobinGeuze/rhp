<?php
var_dump($_POST, $_GET, $_REQUEST);
var_dump(file_get_contents('php://input'));
$f = fopen('php://input', 'r');
var_dump(fread($f, 5), stream_get_contents($f));
