<?php
$d = sys_get_temp_dir() . '/rhp_ziplife_' . getmypid();
@mkdir($d);
chdir($d);
file_put_contents('src.txt', str_repeat("line\n", 50));
// an archive that is never closed is written when the object is freed
function make() { $z = new ZipArchive; $z->open('a.zip', ZipArchive::CREATE); $z->addFromString('x', 'hello'); }
make();
var_dump(file_exists('a.zip'));
$g = new ZipArchive; $g->open('b.zip', ZipArchive::CREATE); $g->addFromString('c:\win.txt', 'w'); $g->addFromString('d/e.txt', 'e');
var_dump(file_exists('b.zip'));
unset($g);
var_dump(file_exists('b.zip'));
// backslashes are ordinary name bytes on extraction
$h = new ZipArchive; $h->open('b.zip'); mkdir('ex');
var_dump($h->extractTo('ex'), scandir('ex'), file_get_contents('ex/c:\win.txt'));
$h->close();
// pending file sources can't be read back before close
$z = new ZipArchive; var_dump($z->open('p.zip', ZipArchive::CREATE));
$z->addFromString('s', str_repeat('a', 300)); $z->addFile('src.txt', 'f');
var_dump($z->statIndex(0)['comp_size'], $z->statIndex(1)['comp_size'], $z->statIndex(1)['size']);
var_dump(strlen($z->getFromIndex(0)), $z->status);
var_dump($z->getFromIndex(1), $z->status, $z->getStatusString());
var_dump($z->getFromName('f'), $z->getStream('f'));
var_dump(@$z->addFile('nope/x')); $e = error_get_last(); echo $e['type'], ': ', $e['message'], "\n";
try { $z->addFile("a\0b"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump($z->close());
$r = new ZipArchive; $r->open('p.zip');
var_dump($r->statIndex(1)['comp_size'] > 0, strlen($r->getFromName('f')));
$r->close();
$y = new ZipArchive; var_dump($y->open($d), $y->open($d, ZipArchive::CREATE));
try { $y->open("a\0b"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(method_exists('ZipArchive', '__construct'), method_exists('ZipArchive', '__chk'), method_exists('ZipArchive', 'open'), method_exists('ZipArchive', 'count'));
// cleanup
foreach (['ex/c:\win.txt', 'ex/d/e.txt', 'src.txt', 'a.zip', 'b.zip', 'p.zip'] as $f) unlink($f);
rmdir('ex/d'); rmdir('ex'); chdir('/'); rmdir($d);
echo "done\n";
