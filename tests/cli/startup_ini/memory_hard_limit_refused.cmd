# A refused startup value warns "in Unknown on line 0" and leaves the
# directive NULL.
noscript
ini memory_hard_limit_percent=50
opt -r
opt echo "start\n"; var_dump(ini_get('memory_hard_limit_percent')); var_dump(ini_get_all(null, false)['memory_hard_limit_percent']); var_dump(error_get_last()); var_dump(ini_set('memory_hard_limit_percent', '200'));
