<?php
// ZipArchive on libzip, limited to what every libzip release php may link
// prints alike: stored (uncompressed) entries, no attributes or newer error
// codes. The rest is in rhp-difftest's zip live test.
$d = sys_get_temp_dir() . '/rhp_golden_zip_basic';
@mkdir($d);
$f = "$d/b.zip";
@unlink($f);

$z = new ZipArchive;
var_dump($z->numFiles, $z->status, $z->filename, $z->lastId, count([$z]));
try { $z->count(); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { $z->open(''); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { $z->open("a\0b"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump($z->open("$d/missing.zip") === ZipArchive::ER_NOENT);
var_dump($z->open($f, ZipArchive::CREATE));
foreach (['a.txt' => 'alpha', 'b.txt' => 'beta beta', 'dir/c.txt' => ''] as $n => $c) {
    var_dump($z->addFromString($n, $c), $z->lastId, $z->setCompressionName($n, ZipArchive::CM_STORE));
}
var_dump($z->addEmptyDir('empty'), $z->addFromString('a.txt', 'x', 0), $z->status === ZipArchive::ER_EXISTS, $z->getStatusString());
$z->clearError();
var_dump($z->status, $z->numFiles, count($z), $z->setArchiveComment('note'), $z->comment);
var_dump($z->setCommentName('b.txt', 'about b'), $z->getCommentName('b.txt'), $z->setMtimeName('a.txt', 1700000000));
try { $z->numFiles = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
var_dump(isset($z->status), empty($z->status), isset($z->nothing));
var_dump($z->close(), $z->status, $z->filename, $z->numFiles);

var_dump($z->open($f, ZipArchive::RDONLY), $z->numFiles, $z->comment);
for ($i = 0; $i < $z->numFiles; $i++) {
    $s = $z->statIndex($i);
    echo $s['index'], ' ', $s['name'], ' ', $s['size'], ' ', $s['comp_size'], ' ', $s['crc'], ' ', $s['comp_method'], "\n";
}
var_dump($z->statName('a.txt')['mtime'], $z->locateName('B.TXT', ZipArchive::FL_NOCASE), $z->locateName('c.txt', ZipArchive::FL_NODIR), $z->locateName('zz'));
var_dump($z->getFromName('a.txt'), $z->getFromName('b.txt', 4), $z->getFromIndex(2), $z->getFromName('nope'), $z->getNameIndex(3), $z->getNameIndex(9));
$s = $z->getStream('b.txt');
var_dump(fread($s, 4), feof($s), stream_get_contents($s), feof($s));
fclose($s);
var_dump(file_get_contents("zip://$f#a.txt"));
var_dump($z->extractTo("$d/out"), file_get_contents("$d/out/b.txt"), is_dir("$d/out/empty"));
$z->close();

$z->open($f);
var_dump($z->deleteName('a.txt'), $z->renameName('b.txt', 'bb.txt'), $z->numFiles, $z->locateName('bb.txt'));
var_dump($z->unchangeIndex(1), $z->getNameIndex(1), $z->close());

$r = @zip_open($f);
while ($e = @zip_read($r)) {
    echo @zip_entry_name($e), ' ', @zip_entry_filesize($e), ' ', @zip_entry_compressionmethod($e), ' ', @zip_entry_read($e, 3), "\n";
}
@zip_close($r);

foreach (['out/a.txt', 'out/b.txt', 'out/dir/c.txt', 'out/dir', 'out/empty', 'out', 'b.zip', ''] as $p) {
    is_dir("$d/$p") ? rmdir("$d/$p") : unlink("$d/$p");
}
echo "done\n";
