<?php
$dir = sys_get_temp_dir() . '/rhp_golden_zip_write';
@mkdir($dir);
$f = $dir . '/t.zip';
@unlink($f);
file_put_contents($dir . '/src.txt', str_repeat("line of text\n", 50));

function dumpStat($s) {
    unset($s['mtime']);
    echo json_encode($s), "\n";
}

$z = new ZipArchive;
var_dump($z->numFiles, $z->status, $z->filename, $z->lastId);
try { $z->addFromString('a', 'b'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { $z->open(''); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump($z->open($dir . '/missing.zip'));
var_dump($z->open($f, ZipArchive::CREATE));
var_dump($z->filename === $f);
var_dump($z->addFromString('a.txt', str_repeat('hello ', 100)), $z->lastId);
var_dump($z->addFromString('b.txt', 'x'));
var_dump($z->addFromString('empty.txt', ''));
var_dump($z->addEmptyDir('dir'), $z->addEmptyDir('dir2/'), $z->addEmptyDir('dir'));
var_dump($z->status, $z->getStatusString());
$z->clearError();
var_dump($z->addFile($dir . '/src.txt', 'sub/src.txt'));
var_dump($z->addFile($dir . '/src.txt', 'part.txt', 5, 10));
var_dump($z->addFromString('a.txt', 'replaced'), $z->numFiles);
var_dump($z->addFromString('a.txt', 'nope', 0), $z->status);
var_dump($z->setExternalAttributesName('b.txt', ZipArchive::OPSYS_UNIX, 0100755 << 16));
var_dump($z->setExternalAttributesName('nope', ZipArchive::OPSYS_UNIX, 0));
var_dump($z->getExternalAttributesName('empty.txt', $o, $a), $o, decoct($a >> 16));
var_dump($z->getExternalAttributesName('dir/', $o, $a), $o, decoct($a >> 16));
dumpStat($z->statName('b.txt'));
var_dump($z->count(), $z->numFiles);
var_dump($z->setArchiveComment('archive comment'), $z->comment);
var_dump($z->setCommentName('b.txt', 'entry comment'));
var_dump($z->close(), $z->numFiles, $z->filename, $z->status);
var_dump(file_exists($f));

$z = new ZipArchive;
var_dump($z->open($f));
var_dump($z->numFiles, $z->getArchiveComment(), $z->comment, $z->getCommentName('b.txt'), $z->getCommentIndex(0));
for ($i = 0; $i < $z->numFiles; $i++) {
    dumpStat($z->statIndex($i));
}
var_dump($z->getExternalAttributesIndex(1, $o, $a), $o, decoct($a >> 16));
var_dump($z->getExternalAttributesName('sub/src.txt', $o, $a), $o, decoct($a >> 16) === decoct(fileperms($dir . '/src.txt')));
var_dump($z->locateName('b.txt'), $z->locateName('B.TXT'), $z->locateName('B.TXT', ZipArchive::FL_NOCASE), $z->locateName('src.txt', ZipArchive::FL_NODIR), $z->locateName('zz'));
var_dump($z->getFromName('a.txt'), $z->getFromName('part.txt'), $z->getFromIndex(1), $z->getFromName('nope'), $z->getFromName('a.txt', 3));
var_dump(strlen($z->getFromName('sub/src.txt')), $z->getFromName('empty.txt'));
var_dump($z->getNameIndex(2), $z->getNameIndex(99), $z->statIndex(99));
$s = $z->getStream('b.txt');
var_dump(is_resource($s), stream_get_contents($s));
fclose($s);
var_dump($z->deleteName('b.txt'), $z->numFiles, $z->getNameIndex(1), $z->locateName('b.txt'), $z->deleteName('b.txt'));
var_dump($z->renameName('a.txt', 'c.txt'), $z->locateName('c.txt'), $z->locateName('a.txt'));
var_dump($z->renameName('c.txt', 'dir/'));
var_dump($z->setCompressionName('part.txt', ZipArchive::CM_STORE));
var_dump($z->close());

$z = new ZipArchive;
var_dump($z->open($f, ZipArchive::RDONLY));
for ($i = 0; $i < $z->numFiles; $i++) {
    echo $z->getNameIndex($i), "\n";
}
$out = $dir . '/out';
var_dump($z->extractTo($out));
var_dump(file_get_contents($out . '/c.txt'), is_dir($out . '/dir'), is_dir($out . '/dir2'), filesize($out . '/sub/src.txt'));
var_dump($z->extractTo($out . '2', 'part.txt'), file_get_contents($out . '2/part.txt'));
var_dump($z->extractTo($out . '3', ['c.txt', 'sub/src.txt']), file_exists($out . '3/c.txt'), file_exists($out . '3/sub/src.txt'), file_exists($out . '3/part.txt'));
$z->close();

var_dump($z->open($f, ZipArchive::CREATE | ZipArchive::EXCL));
file_put_contents($dir . '/bad.zip', 'not a zip file at all, definitely');
var_dump($z->open($dir . '/bad.zip'), $z->open($dir . '/bad.zip', ZipArchive::CREATE));
var_dump($z->open($dir . '/bad.zip', ZipArchive::OVERWRITE), $z->numFiles);
var_dump($z->addFromString('new.txt', 'new'), $z->close(), (new ZipArchive)->open($dir . '/bad.zip'));
var_dump($z->open($f, ZipArchive::OVERWRITE), $z->numFiles, $z->close(), file_exists($f));
var_dump($z->open($f, ZipArchive::CREATE), $z->close(), file_exists($f));
var_dump($z->getStatusString());
var_dump(ZipArchive::isCompressionMethodSupported(ZipArchive::CM_DEFLATE), ZipArchive::isEncryptionMethodSupported(ZipArchive::EM_NONE));
var_dump(ZipArchive::CREATE, ZipArchive::OVERWRITE, ZipArchive::FL_OVERWRITE, ZipArchive::OPSYS_UNIX, ZipArchive::ER_NOZIP, ZipArchive::LENGTH_TO_END);
var_dump($z instanceof Countable);

// cleanup
foreach (['out/c.txt', 'out/part.txt', 'out/empty.txt', 'out/sub/src.txt', 'out2/part.txt', 'out3/c.txt', 'out3/sub/src.txt', 'src.txt', 'bad.zip'] as $p) {
    @unlink("$dir/$p");
}
foreach (['out/sub', 'out/dir', 'out/dir2', 'out', 'out2', 'out3/sub', 'out3', ''] as $p) {
    @rmdir("$dir/$p");
}
var_dump(is_dir($dir));
