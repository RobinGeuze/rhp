<?php
// Prints the build identity of the running php: its release, compile-time
// paths and the versions of the libraries it links. These are the values rhp
// mirrors in crates/rhp-builtins/src/version.rs.
//
//     php -n tools/capture-ref-build.php      # the reference
//     rhp -n tools/capture-ref-build.php      # must print the same
//
// crates/rhp-cli/tests/ref_build.rs runs both and compares them: the
// "release" section whenever php is the same release as rhp, the "build"
// section only against the reference build (Homebrew php on macOS).
// To bump rhp to a new php release, upgrade the reference php, run this
// script and copy the changed values into version.rs.

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

function ext_row(string $ext, string $key, callable $value): void
{
    row($key, extension_loaded($ext) ? $value() : "($ext not loaded)");
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

echo "# build\n";
foreach (['PHP_BUILD_DATE', 'PHP_PREFIX', 'PHP_BINDIR', 'PHP_EXTENSION_DIR',
          'DEFAULT_INCLUDE_PATH'] as $c) {
    row($c, defined($c) ? constant($c) : '(undefined)');
}
row("ini_get('include_path')", ini_get('include_path'));
row("ini_get('extension_dir')", ini_get('extension_dir'));
info_rows(['Build Date']);
foreach (['PCRE_VERSION', 'PCRE_VERSION_MAJOR', 'PCRE_VERSION_MINOR', 'LIBXML_VERSION',
          'LIBXML_DOTTED_VERSION', 'ICONV_VERSION', 'MB_ONIGURUMA_VERSION',
          'OPENSSL_VERSION_TEXT', 'OPENSSL_VERSION_NUMBER', 'INTL_ICU_VERSION',
          'INTL_ICU_DATA_VERSION', 'GMP_VERSION'] as $c) {
    row($c, defined($c) ? constant($c) : '(undefined)');
}
ext_row('zip', 'ZipArchive::LIBZIP_VERSION', fn () => ZipArchive::LIBZIP_VERSION);
ext_row('curl', 'curl_version()', function () {
    $v = curl_version();
    // The array without feature_list (derived from 'features').
    unset($v['feature_list']);
    return $v;
});
