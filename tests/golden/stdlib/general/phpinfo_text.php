<?php
var_dump(INFO_GENERAL, INFO_CREDITS, INFO_CONFIGURATION, INFO_MODULES, INFO_ENVIRONMENT, INFO_VARIABLES, INFO_LICENSE, INFO_ALL);
var_dump(CREDITS_GROUP, CREDITS_GENERAL, CREDITS_SAPI, CREDITS_MODULES, CREDITS_DOCS, CREDITS_FULLPAGE, CREDITS_QA, CREDITS_ALL);
var_dump(phpinfo(INFO_LICENSE));
ob_start();
phpinfo(INFO_GENERAL);
$s = ob_get_clean();
// composer's DiagnoseCommand parse
// (Homebrew's php has the row, Ubuntu's does not.)
$has = (bool) preg_match('{Configure Command(?: *</td><td class="v">| *=> *)(.*?)(?:</td>|$)}m', $s, $m);
var_dump(!$has || !str_contains($m[1], '--enable-sigchild'));
$lines = explode("\n", $s);
var_dump($lines[0], $lines[1] === 'PHP Version => ' . PHP_VERSION, $lines[2]);
var_dump(str_contains($s, "\nPHP Integer Size => 64 bits\n"), str_contains($s, "\nServer API => Command Line Interface\n"));
var_dump(str_contains($s, "\nThis program makes use of the Zend Scripting Language Engine:\n"));
var_dump(preg_match('/^System => (\S+) /m', $s, $m), $m[1] === PHP_OS, php_uname('s') === PHP_OS);
var_dump(php_uname() === implode(' ', [php_uname('s'), php_uname('n'), php_uname('r'), php_uname('v'), php_uname('m')]));
foreach (['', 'x', 'ab'] as $mode) {
    try {
        php_uname($mode);
    } catch (ValueError $e) {
        echo $e->getMessage(), "\n";
    }
}
ob_start();
phpinfo(INFO_CONFIGURATION);
$c = ob_get_clean();
var_dump(str_contains($c, "Configuration\n\nPHP Core\n\nDirective => Local Value => Master Value\nallow_url_fopen => On => On\n"));
