<?php
// ReflectionExtension: info() prints the module's phpinfo section
// (php_info_print_module), and the getters list what the module registers.
foreach (['bz2', 'ctype', 'bcmath', 'filter', 'Reflection', 'tokenizer'] as $e) {
    (new ReflectionExtension($e))->info();
}
foreach (['standard', 'dom', 'Phar', 'ctype'] as $e) {
    var_dump((new ReflectionExtension($e))->getDependencies());
}
var_dump((new ReflectionExtension('bcmath'))->getINIEntries());
var_dump((new ReflectionExtension('json'))->getClassNames());
// (php lists both in registration order; compare as sets)
$c = (new ReflectionExtension('json'))->getConstants();
ksort($c);
var_dump($c);
$f = array_map(fn ($r) => get_class($r) . ' ' . $r->name, (new ReflectionExtension('json'))->getFunctions());
ksort($f);
var_dump($f);
var_dump(count((new ReflectionExtension('ctype'))->getFunctions()));
