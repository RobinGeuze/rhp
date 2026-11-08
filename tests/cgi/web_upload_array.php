<?php
$f = $_FILES;
// tmp_name is random.
array_walk_recursive($f['f']['tmp_name'], function (&$v) { $v = $v === '' ? '' : 'TMP'; });
var_dump($f, $_POST);
