<?php
// An autoloader that requires one file per class.
spl_autoload_register(function ($class) {
    $f = __DIR__ . '/spl_autoload_files_' . $class . '.inc';
    if (file_exists($f)) require $f;
});
echo (new Mailer())->send('x'), "\n";
echo Logger::log('y'), "\n";
var_dump(class_exists('Mailer'));
echo count(spl_autoload_functions()), "\n";
