<?php
// php_plain_files_rename takes a leading file:// off both names.
$d = sys_get_temp_dir() . '/rhp_rename_file_url_' . getmypid();
@mkdir($d);
file_put_contents("$d/a", 'x');
var_dump(rename("file://$d/a", "FILE://$d/b"), file_get_contents("$d/b"));
var_dump(copy("file://$d/b", "file://$d/c"), file_exists("$d/c"));
var_dump(unlink("file://$d/b"), unlink("$d/c"), rmdir($d));
