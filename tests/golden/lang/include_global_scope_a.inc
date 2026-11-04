<?php
namespace Inc;
// $argv/$argc and variables of the includer are globals here even though the
// including file never names them.
var_dump(is_array($argv), is_int($argc), $fromMain ?? 'unset-main');
$setByA = 'from a';
