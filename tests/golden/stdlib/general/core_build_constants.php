<?php
// The Core constants php defines, in php's order, with the host-independent
// ones' values (the build paths differ per build; only their types and shape
// are checked).
$core = get_defined_constants(true)['Core'];
echo implode("\n", array_keys($core)), "\n";
var_dump(PHP_DEBUG, PHP_ZTS, ZEND_THREAD_SAFE, ZEND_DEBUG_BUILD, PHP_SHLIB_SUFFIX, PHP_FD_SETSIZE);
var_dump(str_starts_with(ZEND_VM_KIND, 'ZEND_VM_KIND_'), TRUE, FALSE, NULL, constant('TRUE'), defined('NULL'));
var_dump(PHP_CLI_PROCESS_TITLE);
foreach (['UPLOAD_ERR_OK', 'UPLOAD_ERR_INI_SIZE', 'UPLOAD_ERR_FORM_SIZE', 'UPLOAD_ERR_PARTIAL', 'UPLOAD_ERR_NO_FILE',
          'UPLOAD_ERR_NO_TMP_DIR', 'UPLOAD_ERR_CANT_WRITE', 'UPLOAD_ERR_EXTENSION'] as $c) {
    echo $c, ' = ', constant($c), "\n";
}
foreach (['PHP_BUILD_PROVIDER', 'PHP_PREFIX', 'PHP_BINDIR', 'PHP_SBINDIR', 'PHP_MANDIR', 'PHP_LIBDIR', 'PHP_DATADIR',
          'PHP_SYSCONFDIR', 'PHP_LOCALSTATEDIR', 'PHP_CONFIG_FILE_PATH', 'PHP_CONFIG_FILE_SCAN_DIR',
          'PEAR_INSTALL_DIR', 'PEAR_EXTENSION_DIR', 'PHP_EXTENSION_DIR', 'DEFAULT_INCLUDE_PATH'] as $c) {
    $v = constant($c);
    echo $c, ': ', gettype($v), ' ', $v !== '' ? 'non-empty' : 'empty', "\n";
}
var_dump(PEAR_EXTENSION_DIR === PHP_EXTENSION_DIR, str_starts_with(PHP_LIBDIR, PHP_PREFIX));
var_dump(str_ends_with(PHP_CONFIG_FILE_SCAN_DIR, '/conf.d'), str_starts_with(PHP_CONFIG_FILE_SCAN_DIR, PHP_CONFIG_FILE_PATH));
ob_start();
phpinfo(INFO_GENERAL);
$info = ob_get_clean();
var_dump(str_contains($info, "\nBuild Provider => " . PHP_BUILD_PROVIDER . "\n"));
var_dump(str_contains($info, "\nConfiguration File (php.ini) Path => " . PHP_CONFIG_FILE_PATH . "\n"));
$z = array_map('intval', explode('.', ZLIB_VERSION)) + [0, 0, 0, 0];
var_dump(ZLIB_VERNUM === ($z[0] << 12 | ($z[1] & 15) << 8 | ($z[2] & 15) << 4 | ($z[3] & 15)));
