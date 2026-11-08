# php-cli's HARDCODED_INI is applied after php.ini (which cannot change
# those settings) and before -d (which can).
no-n
opt -c
opt {DIR}/hardcoded.ini
ini max_input_time=5
noscript
opt -r
opt foreach (['max_execution_time', 'output_buffering', 'html_errors', 'implicit_flush', 'max_input_time', 'memory_limit'] as $k) { var_dump(ini_get($k), get_cfg_var($k)); }
