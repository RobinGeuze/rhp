<?php
// use_include_path: php_resolve_path, then the plain name
chdir(__DIR__);
@mkdir('include_path_open.dir');
file_put_contents('include_path_open.dir/inc.tmp', "from include_path\n");
set_include_path(__DIR__ . '/include_path_open.dir');
echo file_get_contents('inc.tmp', true);
readfile('inc.tmp', true);
echo implode('', file('inc.tmp', FILE_USE_INCLUDE_PATH));
$h = fopen('inc.tmp', 'r', true); echo fgets($h); fclose($h);
var_dump(basename(stream_resolve_include_path('inc.tmp')));
// a new file goes to the working directory, not the include path
file_put_contents('include_path_open_new.tmp', 'x', FILE_USE_INCLUDE_PATH);
var_dump(file_exists('include_path_open_new.tmp'), file_exists('include_path_open.dir/include_path_open_new.tmp'));
unlink('include_path_open_new.tmp');
file_put_contents('inc.tmp', "appended\n", FILE_USE_INCLUDE_PATH | FILE_APPEND);
echo file_get_contents('include_path_open.dir/inc.tmp');
var_dump(@file_get_contents('missing.tmp', true));
unlink('include_path_open.dir/inc.tmp');
rmdir('include_path_open.dir');
