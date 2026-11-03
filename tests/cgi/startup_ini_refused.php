<?php
echo "start\n";
var_dump(ini_get('memory_hard_limit_percent'));
var_dump(ini_get_all(null, false)['memory_hard_limit_percent']);
var_dump(ini_set('memory_hard_limit_percent', '200'));
var_dump(ini_get('memory_hard_limit_percent'));
