<?php
// php-cgi run from a shell (no CGI variables) has dl().
var_dump(function_exists('dl'), is_callable('dl'));
