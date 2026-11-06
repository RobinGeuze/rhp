<?php
// open_basedir set at run time: ini_set may only tighten it, and every
// path-taking function checks its path (warnings with the base dir shown).
$base = sys_get_temp_dir() . '/rhp_open_basedir_' . getmypid();
@mkdir($base);
$base = realpath($base);
foreach (['in', 'in/sub', 'out'] as $d) {
    @mkdir("$base/$d");
}
file_put_contents("$base/in/ok.txt", "ok\n");
file_put_contents("$base/out/bad.txt", "bad\n");
file_put_contents("$base/out/x.php", "<?php echo 'included';");
file_put_contents("$base/in/conf.ini", "a=1\n");
@symlink("$base/out", "$base/in/lnk");
@symlink("$base/nowhere/zz", "$base/in/broken");
chdir("$base/in");
set_include_path('.');

set_error_handler(function ($no, $msg) use ($base) {
    echo str_replace($base, 'BASE', $msg), "\n";
    return true;
});
function show($label, $v) {
    global $base;
    echo $label, ': ', str_replace($base, 'BASE', var_export($v, true)), "\n";
}

show('first set', ini_set('open_basedir', "$base/in/"));
show('ini_get keeps the value as given', ini_get('open_basedir'));
show('loosen', ini_set('open_basedir', $base));
show('unset', ini_set('open_basedir', ''));
show('relative entry', ini_set('open_basedir', "sub:."));
show('after', ini_get('open_basedir'));
show('restore fails', ini_restore('open_basedir'));
show('still', ini_get('open_basedir'));

// "." is the working directory, so chdir moves it; tighten back to "in"
show('set in', ini_set('open_basedir', "$base/in"));
show('symlink out', file_exists('lnk'));
show('symlink out, slash', file_exists('lnk/'));
show('broken link', file_exists('broken'));
show('missing dirs inside', file_exists('sub/a/b/../../x'));
show('dotdot out', is_dir('sub/../..'));
show('realpath out', realpath('..'));
show('realpath in', basename(realpath('sub')));
show('fopen', fopen('../out/bad.txt', 'r'));
show('get', file_get_contents('lnk/bad.txt'));
show('include', include '../out/x.php');
show('parse_ini_file', parse_ini_file('conf.ini'));
show('parse_ini_file out', parse_ini_file('../out/bad.txt'));
show('highlight out', highlight_file('../out/x.php', true));
show('copy in', copy('ok.txt', 'ok2.txt'));
show('copy out', copy('ok.txt', '../out/c.txt'));
show('copy from out', copy('../out/bad.txt', 'c.txt'));
show('rename out', rename('ok2.txt', '../out/ok2.txt'));
show('unlink', unlink('ok2.txt'));
show('mkdir', mkdir('../out/n'));
show('mkdir recursive', mkdir('sub/../../out/n/m', 0777, true));
show('touch', touch('../out/t'));
show('chmod', chmod('../out/bad.txt', 0644));
show('symlink', symlink('../out', 'lnk2'));
show('link', link('../out/bad.txt', 'h'));
show('readlink', readlink('lnk'));
show('linkinfo', linkinfo('../out/bad.txt'));
show('tempnam', tempnam('../out', 'x'));
show('disk_free_space', disk_free_space('..'));
show('glob', glob("$base/*"));
show('glob in', array_map('basename', glob("$base/in/*.txt")));
show('scandir', scandir('..'));
show('opendir', opendir('../out'));
show('chdir', chdir('..'));
show('error_log', error_log('x', 3, '../out/log'));
show('ini error_log', ini_set('error_log', "$base/out/log"));
show('stat', stat('/'));
show('lstat', lstat('../out'));
show('filesize', filesize('../out/bad.txt'));
show('is_writable', is_writable('../out'));

restore_error_handler();
// (open_basedir binds the script, not a shell)
exec('rm -rf ' . escapeshellarg($base));
echo "done\n";
