<?php
// php_spl.c's default autoloader: spl_autoload_register() with no callback
// registers spl_autoload(), which includes lowercase class name + extension
// from the include path.
@mkdir(sys_get_temp_dir() . '/rhp_spl_autoload/ns', 0777, true);
$dir = sys_get_temp_dir() . '/rhp_spl_autoload';
file_put_contents("$dir/autofoo.inc", '<?php class AutoFoo { }');
file_put_contents("$dir/ns/autobar.php", '<?php namespace Ns; class AutoBar { }');
set_include_path($dir);
var_dump(spl_autoload_extensions());
spl_autoload_register();
var_dump(spl_autoload_functions(), get_class(new AutoFoo), get_class(new Ns\AutoBar), class_exists('NoSuchClass'));
var_dump(spl_autoload_extensions('.php'), class_exists('AutoFoo2'));
try { spl_object_id(null); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
unlink("$dir/autofoo.inc"); unlink("$dir/ns/autobar.php"); rmdir("$dir/ns"); rmdir($dir);
