<?php
// Prints the build identity of the running php: its release, compile-time
// paths and the versions of the libraries it links. These are the values rhp
// mirrors in crates/rhp-builtins/src/version.rs.
//
//     php -n tools/capture-ref-build.php      # the reference
//     rhp -n tools/capture-ref-build.php      # must print the same
//
// crates/rhp-cli/tests/ref_build.rs runs both and compares them: the
// "release" section whenever php is the same release as rhp, the "linked"
// section (libraries rhp links itself) against any php 8.5, the "build"
// section only against the reference build (Homebrew php on macOS).
// To bump rhp to a new php release, upgrade the reference php, run this
// script and copy the changed values into version.rs.
//
//     php -n tools/capture-ref-build.php tables
//
// prints the reference build's ini directives and constant modules instead
// (php-cgi: `php-cgi -n -q -f tools/capture-ref-build.php -- tables`), which
// `rhp-difftest gen dumps` turns into ini_table.rs and const_modules.rs.

function row(string $key, mixed $value): void
{
    echo $key, ' = ', var_export($value, true), "\n";
}

/** The rows of `phpinfo(INFO_GENERAL)` named in $keys (and the Zend line). */
function info_rows(array $keys, bool $zend = false): void
{
    ob_start();
    phpinfo(INFO_GENERAL);
    $info = ob_get_clean();
    foreach (explode("\n", $info) as $line) {
        $parts = explode(' => ', $line, 2);
        if (count($parts) === 2 && in_array($parts[0], $keys, true)) {
            row("phpinfo[{$parts[0]}]", $parts[1]);
        }
        if ($zend && str_starts_with($line, 'Zend Engine v')) {
            row('phpinfo[Zend Engine]', $line);
        }
    }
}

/** The rows of `phpinfo(INFO_MODULES)` named in $keys. */
function module_rows(array $keys): void
{
    ob_start();
    phpinfo(INFO_MODULES);
    $info = ob_get_clean();
    foreach (explode("\n", $info) as $line) {
        $parts = explode(' => ', $line, 2);
        if (count($parts) === 2 && in_array($parts[0], $keys, true)) {
            row("phpinfo[{$parts[0]}]", $parts[1]);
        }
    }
}

function ext_row(string $ext, string $key, callable $value): void
{
    row($key, extension_loaded($ext) ? $value() : "($ext not loaded)");
}

if (($argv[1] ?? '') === 'tables') {
    // ini_get_all() with each directive's owning module: the (lowercase)
    // name ini_get_all() takes; "core" lists every directive
    $owner = [];
    foreach (array_merge(get_loaded_extensions(), get_loaded_extensions(true)) as $ext) {
        $ext = strtolower($ext);
        foreach ($ext === 'core' ? [] : ini_get_all($ext, true) as $name => $_) {
            $owner[$name] = $ext;
        }
    }
    echo "# ini\n";
    foreach (ini_get_all(null, true) as $name => $v) {
        echo $owner[$name] ?? 'core', "\t", $name, "\t", $v['access'], "\t",
            var_export($v['global_value'], true), "\n";
    }
    echo "# constants\n";
    foreach (get_defined_constants(true) as $module => $constants) {
        echo $module, "\t", implode(' ', array_keys($constants)), "\n";
    }
    echo "# paths\n";
    row('PHP_EXTENSION_DIR', PHP_EXTENSION_DIR);
    row('DEFAULT_INCLUDE_PATH', DEFAULT_INCLUDE_PATH);
    exit;
}

echo "# release\n";
foreach (['PHP_VERSION', 'PHP_MAJOR_VERSION', 'PHP_MINOR_VERSION', 'PHP_RELEASE_VERSION',
          'PHP_EXTRA_VERSION', 'PHP_VERSION_ID'] as $c) {
    row($c, constant($c));
}
row('zend_version()', zend_version());
row('phpversion()', phpversion());
foreach (['Core', 'standard', 'date', 'pcre', 'json', 'hash', 'zip', 'dom'] as $ext) {
    row("phpversion('$ext')", phpversion($ext));
}
info_rows(['PHP Version', 'PHP API', 'PHP Extension', 'Zend Extension',
           'Zend Extension Build', 'PHP Extension Build'], true);
$all = ini_get_all(null, false);
row("ini_get_all()[memory_hard_limit_percent]",
    array_key_exists('memory_hard_limit_percent', $all) ? $all['memory_hard_limit_percent'] : '(absent)');

// Libraries rhp links itself (docs/design/libraries.md): the versions must
// be the ones php links, on every machine.
echo "# linked\n";
foreach (['PCRE_VERSION', 'PCRE_VERSION_MAJOR', 'PCRE_VERSION_MINOR',
          'LIBXML_VERSION', 'LIBXML_DOTTED_VERSION', 'LIBXML_LOADED_VERSION',
          'INTL_ICU_VERSION', 'INTL_ICU_DATA_VERSION'] as $c) {
    row($c, defined($c) ? constant($c) : '(undefined)');
}
// The OpenSSL loaded at run time. OPENSSL_VERSION_TEXT/NUMBER are the headers
// each binary was compiled against: a Homebrew bottle built before an
// openssl@4 update reports the older headers, so they are compared nowhere.
module_rows(['OpenSSL Library Version']);
ext_row('curl', 'curl_version()', function () {
    $v = curl_version();
    // The array without feature_list (derived from 'features').
    unset($v['feature_list']);
    return $v;
});
foreach (['ICONV_IMPL', 'ICONV_VERSION', 'GMP_VERSION'] as $c) {
    row($c, defined($c) ? constant($c) : '(undefined)');
}
ext_row('zip', 'ZipArchive::LIBZIP_VERSION', fn () => ZipArchive::LIBZIP_VERSION);
module_rows(['GMP version']);

echo "# build\n";
foreach (['PHP_BUILD_DATE', 'PHP_BUILD_PROVIDER', 'PHP_PREFIX', 'PHP_BINDIR', 'PHP_SBINDIR',
          'PHP_MANDIR', 'PHP_LIBDIR', 'PHP_DATADIR', 'PHP_SYSCONFDIR', 'PHP_LOCALSTATEDIR',
          'PHP_CONFIG_FILE_PATH', 'PHP_CONFIG_FILE_SCAN_DIR', 'PHP_SHLIB_SUFFIX',
          'PHP_EXTENSION_DIR', 'DEFAULT_INCLUDE_PATH', 'PEAR_INSTALL_DIR',
          'PEAR_EXTENSION_DIR'] as $c) {
    row($c, defined($c) ? constant($c) : '(undefined)');
}
row("ini_get('include_path')", ini_get('include_path'));
row("ini_get('extension_dir')", ini_get('extension_dir'));
info_rows(['Build Date', 'Build System', 'Build Provider', 'Configuration File (php.ini) Path']);
foreach (['MB_ONIGURUMA_VERSION', 'ZLIB_VERSION', 'ZLIB_VERNUM'] as $c) {
    row($c, defined($c) ? constant($c) : '(undefined)');
}
