<?php
// php-cgi registers dl() only when run from a shell, not for a request.
var_dump(function_exists('dl'), in_array('dl', get_extension_funcs('standard')));
