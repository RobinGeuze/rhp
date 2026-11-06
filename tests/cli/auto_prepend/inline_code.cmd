# -r code runs without auto_prepend_file (php doesn't use
# php_execute_script for it).
ini auto_prepend_file={DIR}/pre.inc
noscript
opt -r
opt var_dump($pre ?? null);
