<?php
// include of a file:// URL is the plain path it names (php_resolve_path);
// include "" throws (php_stream_open_wrapper_ex).
$f = __DIR__ . '/include_file_url_empty.tmp.inc';
file_put_contents($f, '<?php return basename(__FILE__);');
var_dump(include 'file://' . $f);
var_dump(require_once 'FILE://' . $f);
foreach (['include', 'require'] as $k) {
    try { $k === 'include' ? include '' : require ''; } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
}
unlink($f);
