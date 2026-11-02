<?php
// Shared by the upload_* fixtures: $_FILES with each tmp_name replaced by
// what can be compared across engines (temp names are random).
function upload_desc($v)
{
    if (!is_string($v) || $v === '') {
        return $v;
    }
    $dir = ini_get('upload_tmp_dir');
    $want = realpath($dir !== '' && is_dir($dir) ? $dir : sys_get_temp_dir());
    return sprintf(
        '%s %s uploaded=%s exists=%s contents=%s',
        preg_match('/^php[0-9a-v]{13}[0-9A-Za-z]{6}$/', basename($v)) ? 'name-ok' : 'name-bad:' . basename($v),
        dirname($v) === $want ? 'dir-ok' : 'dir-bad:' . dirname($v),
        var_export(is_uploaded_file($v), true),
        var_export(file_exists($v), true),
        json_encode(file_exists($v) ? (strlen($c = file_get_contents($v)) > 64 ? strlen($c) . ' bytes ' . md5($c) : $c) : null)
    );
}

function upload_mask($v)
{
    if (is_array($v)) {
        foreach ($v as $k => $x) {
            $v[$k] = upload_mask($x);
        }
        return $v;
    }
    return upload_desc($v);
}

function upload_files(array $files)
{
    foreach ($files as $n => $info) {
        if (is_array($info) && array_key_exists('tmp_name', $info)) {
            $files[$n]['tmp_name'] = upload_mask($info['tmp_name']);
        }
    }
    return $files;
}
