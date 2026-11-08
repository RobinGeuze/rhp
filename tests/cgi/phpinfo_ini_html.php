<?php
// phpinfo's HTML module tables: the module rows (php_info_print_table_row)
// and the directive rows (display_ini_entries) are formatted differently.
foreach (['zlib', 'pcre'] as $e) {
    (new ReflectionExtension($e))->info();
}
