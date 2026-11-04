# The output layer's startup ini: output_buffering and output_handler.
noscript
opt -r
opt echo "abc\n"; var_dump(ini_get("output_buffering"), ob_get_level(), ob_list_handlers());
