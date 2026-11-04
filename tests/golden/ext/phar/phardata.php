<?php
// PharData: tar / tar.gz / tar.bz2 archives (composer archive, TarDownloader,
// Tar::getComposerJson).
$d = sys_get_temp_dir() . '/rhp_phardata_' . getmypid();
@mkdir($d . '/src/pkg/lib', 0777, true);
file_put_contents($d . '/src/pkg/composer.json', '{"name":"a/b"}');
file_put_contents($d . '/src/pkg/lib/x.php', '<?php');
$n = new PharData($d . '/dist.tar');
$map = $n->buildFromIterator(new ArrayIterator([$d . '/src/pkg/composer.json' => new SplFileInfo($d . '/src/pkg/composer.json'), $d . '/src/pkg/lib/x.php' => new SplFileInfo($d . '/src/pkg/lib/x.php')]), $d . '/src');
var_dump(array_keys($map));
$n->addEmptyDir('pkg/empty');
$n->addFromString('pkg/README', 'readme');
foreach ([Phar::GZ, Phar::BZ2] as $c) {
    $z = $n->compress($c);
    echo basename($z->getPath()), "\n";
}
foreach (['dist.tar', 'dist.tar.gz', 'dist.tar.bz2'] as $f) {
    $p = new PharData($d . '/' . $f);
    var_dump($p->valid(), count($p), isset($p['composer.json']), isset($p['pkg/composer.json']), isset($p['pkg']));
    foreach ($p as $k => $fi) { echo basename($k), ' ', $fi->getBasename(), ' ', var_export($fi->isDir(), true), "\n"; }
    echo $p['pkg/composer.json']->getContent(), "\n";
}
$p = new PharData($d . '/dist.tar.gz');
$p->extractTo($d . '/out', null, true);
$list = scandir($d . '/out/pkg'); echo implode(',', $list), "\n";
echo file_get_contents($d . '/out/pkg/lib/x.php'), "\n";
try { $p->extractTo($d . '/out'); } catch (PharException $e) { echo get_class($e), ': ', str_replace([realpath($d), $d], 'D', $e->getMessage()), "\n"; }
try { $p['nope']; } catch (BadMethodCallException $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { $p['pkg']->getContent(); } catch (BadMethodCallException $e) { echo str_replace([realpath($d), $d], 'D', $e->getMessage()), "\n"; }
var_dump(PharData::canCompress(Phar::GZ), PharData::canCompress(Phar::BZ2));
// cleanup
$it = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($d, FilesystemIterator::SKIP_DOTS), RecursiveIteratorIterator::CHILD_FIRST);
foreach ($it as $f) { $f->isDir() ? rmdir($f->getPathname()) : unlink($f->getPathname()); }
rmdir($d);
var_dump(file_exists($d));
