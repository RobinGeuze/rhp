<?php
// Diagnostics print without the (machine-specific) script path.
set_error_handler(function ($no, $msg) { echo "[$no] $msg\n"; return true; });
$i = new SplFileInfo("foo.txt"); var_dump($i->getPath(), $i->getFilename(), $i->getPathname(), $i->getExtension(), $i->getBasename(".txt"));
$i = new SplFileInfo("/a/b/"); var_dump($i->getPath(), $i->getFilename(), $i->getPathname(), (string)$i);
$i = new SplFileInfo("/"); var_dump($i->getPath(), $i->getFilename(), $i->getPathname());
$i = new SplFileInfo("a/b.tar.gz"); var_dump($i->getPath(), $i->getFilename(), $i->getExtension(), $i->getBasename(), $i->getPathInfo()->getPathname(), $i->getFileInfo()->getFilename());
$i = new SplFileInfo("/nonexist"); try { $i->getSize(); } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; } try { $i->getType(); } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; } var_dump($i->getRealPath(), $i->isFile());
try { $i->openFile(); } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { new SplFileObject("/nonexist"); } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { new DirectoryIterator("/nonexist"); } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { new DirectoryIterator(""); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { new FilesystemIterator("/nonexist"); } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { new SplFileObject(sys_get_temp_dir()); } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
$d = sys_get_temp_dir() . "/rhp_spl_" . getmypid(); @mkdir($d); file_put_contents("$d/f.txt", "l1\nl2\r\n\nl4");
$f = new SplFileObject("$d/f.txt"); foreach ($f as $k => $l) var_dump($k, $l);
$f->setFlags(SplFileObject::DROP_NEW_LINE | SplFileObject::SKIP_EMPTY | SplFileObject::READ_AHEAD); foreach ($f as $k => $l) var_dump($k, $l);
$f->setFlags(0); $f->seek(2); var_dump($f->key(), $f->current()); var_dump($f->fgets(), $f->key(), $f->eof(), $f->valid(), $f->getPath() === $d, $f->getFilename(), (string)$f);
$f->rewind(); var_dump($f->fgetc(), $f->ftell(), $f->fread(3), $f->fstat()["size"], $f->getFlags(), $f->getMaxLineLen());
$f->setMaxLineLen(2); $f->rewind(); var_dump($f->fgets(), $f->fgets());
try { $f->fread(0); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { $f->seek(-1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
$t = new SplTempFileObject(); var_dump($t->getPathname(), $t->getFilename(), $t->getPath(), $t->fwrite("a,b\n\"c\nd\",e\n"), $t->rewind(), $t->fgetcsv(",", "\"", "\\"), $t->fgetcsv(",", "\"", "\\"), $t->key());
$t->setFlags(SplFileObject::READ_CSV); $t->rewind(); foreach ($t as $k => $row) var_dump($k, $row);
var_dump($t->getCsvControl(), $t->fputcsv(["x y", "z"]), (new SplFileInfo("$d/f.txt"))->openFile()->fgets());
$t->setCsvControl(';', "'", ''); var_dump($t->getCsvControl());
$w = new SplFileObject("$d/w.txt", "w+"); var_dump($w->fwrite("hello\nworld\n"), $w->fwrite("xyz", 2), $w->fflush(), $w->ftruncate(8), $w->rewind(), $w->current(), $w->fseek(0, SEEK_END), $w->ftell());
$w = null; var_dump(file_get_contents("$d/w.txt"));
$t2 = new SplTempFileObject(-1); var_dump($t2->getFilename()); $t3 = new SplTempFileObject(1024); var_dump($t3->getFilename());
foreach (new SplFileObject("$d/f.txt") as $n => $line) { if ($n > 1) break; echo $n, ':', trim($line), ' '; } echo "\n";
var_dump(count(iterator_to_array(new GlobIterator("$d/*.txt"))), (new GlobIterator("$d/*.txt"))->count());
foreach (new GlobIterator("$d/*.txt", FilesystemIterator::KEY_AS_FILENAME) as $k => $fi) echo $k, ' ', get_class($fi), ' ', $fi->getFilename(), "\n";
$fsi = new FilesystemIterator($d, FilesystemIterator::KEY_AS_FILENAME | FilesystemIterator::CURRENT_AS_PATHNAME); $all = iterator_to_array($fsi); ksort($all); var_dump(array_map(fn($p) => basename($p), $all), $fsi->getFlags());
$di = new DirectoryIterator($d); $names = []; foreach ($di as $k => $e) { if (!$e->isDot()) $names[] = $e->getFilename() . '|' . $e->getExtension() . '|' . ($e->isFile() ? 'f' : 'd') . '|' . $e->getSize(); } sort($names); var_dump($names);
unlink("$d/f.txt"); unlink("$d/w.txt"); rmdir($d);
