//! ext/zip against php. rhp links the libzip php links, so error codes and
//! texts, compressed sizes, encryption, the constants a release defines and
//! its stream behaviour match the php on the same machine, but differ
//! between libzip releases (the mac's 1.12.1 and CI's 1.11.4): these cannot
//! be goldens. Each case works in a directory of its own under the system
//! temp dir and prints no paths. Skips without `php`.

use rhp_difftest::{diff_source, php_available};

fn check(name: &str, body: &str) {
    if !php_available() {
        eprintln!("[{name}] no php — skipping");
        return;
    }
    // The pid keeps concurrent checkouts running this test apart.
    let dir = format!("{name}_{}", std::process::id());
    let src = PRELUDE.replace("%NAME%", &dir) + body + EPILOGUE;
    if let Some(why) = diff_source(src.as_bytes()).failure() {
        panic!("[{name}] {why}");
    }
}

/// Prints diagnostics (the harness runs php with error_reporting=0), and
/// gives the case an empty directory `$d`.
const PRELUDE: &str = r#"<?php
function eh($no, $str) {
    echo "W[$no]: ", str_replace($GLOBALS['d'], 'D', $str), "\n";
    return true;
}
set_error_handler('eh');
$d = sys_get_temp_dir() . '/rhp_zip_%NAME%';
function rmtree($p) {
    if (is_dir($p) && !is_link($p)) {
        foreach (scandir($p) as $e) { if ($e !== '.' && $e !== '..') rmtree("$p/$e"); }
        rmdir($p);
    } elseif (file_exists($p) || is_link($p)) {
        unlink($p);
    }
}
rmtree($d);
mkdir($d);
function stat_line($s) { unset($s['mtime']); return json_encode($s); }
"#;

const EPILOGUE: &str = "\nrmtree($d);\n";

#[test]
fn archive_write_and_read_match_php() {
    check(
        "write",
        r#"
$f = "$d/t.zip";
file_put_contents("$d/src.txt", str_repeat("line of text\n", 50));
$z = new ZipArchive;
var_dump($z->numFiles, $z->status, $z->filename, $z->lastId);
try { $z->addFromString('a', 'b'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { $z->open(''); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump($z->open("$d/missing.zip"), $z->open($f, ZipArchive::CREATE));
var_dump($z->addFromString('a.txt', str_repeat('hello ', 100)), $z->lastId);
var_dump($z->addFromString('b.txt', 'x'), $z->addFromString('empty.txt', ''));
var_dump($z->addEmptyDir('dir'), $z->addEmptyDir('dir2/'), $z->addEmptyDir('dir'));
var_dump($z->status, $z->getStatusString());
$z->clearError();
var_dump($z->addFile("$d/src.txt", 'sub/src.txt'), $z->addFile("$d/src.txt", 'part.txt', 5, 10));
var_dump($z->addFromString('a.txt', 'replaced'), $z->numFiles, $z->addFromString('a.txt', 'nope', 0), $z->status);
var_dump($z->setExternalAttributesName('b.txt', ZipArchive::OPSYS_UNIX, 0100755 << 16));
var_dump($z->getExternalAttributesName('empty.txt', $o, $a), $o, decoct($a >> 16));
var_dump($z->setArchiveComment('archive comment'), $z->comment, $z->setCommentName('b.txt', 'entry comment'));
var_dump($z->setCompressionName('a.txt', ZipArchive::CM_BZIP2), $z->setCompressionName('b.txt', ZipArchive::CM_XZ));
var_dump($z->setCompressionIndex(0, 12345));
var_dump($z->setMtimeName('b.txt', 1700000000));
var_dump($z->close(), $z->numFiles, $z->filename, $z->status, $z->getStatusString());

$z = new ZipArchive;
var_dump($z->open($f), $z->numFiles, $z->comment, $z->getCommentName('b.txt'));
for ($i = 0; $i < $z->numFiles; $i++) { echo stat_line($z->statIndex($i)), "\n"; }
var_dump($z->statName('b.txt')['mtime']);
var_dump($z->locateName('B.TXT', ZipArchive::FL_NOCASE), $z->locateName('src.txt', ZipArchive::FL_NODIR));
var_dump($z->getFromName('a.txt'), $z->getFromName('part.txt'), $z->getFromName('a.txt', 3), strlen($z->getFromName('sub/src.txt')));
var_dump($z->deleteName('b.txt'), $z->renameName('a.txt', 'c.txt'), $z->unchangeName('c.txt'), $z->unchangeAll());
var_dump($z->extractTo("$d/out"), file_get_contents("$d/out/a.txt"), is_dir("$d/out/dir2"), filesize("$d/out/sub/src.txt"));
var_dump($z->extractTo("$d/out2", ['part.txt', 'nope']), $z->extractTo("$d/out3", 'part.txt'));
$z->close();

file_put_contents("$d/bad.zip", 'not a zip file at all, definitely');
file_put_contents("$d/junk.zip", "PK\x03\x04garbage");
file_put_contents("$d/empty.zip", '');
var_dump($z->open("$d/bad.zip"), $z->open("$d/junk.zip"), $z->open("$d/empty.zip"), $z->open($d), $z->open($f, ZipArchive::EXCL | ZipArchive::CREATE));
var_dump($z->open("$d/bad.zip", ZipArchive::OVERWRITE), $z->numFiles, $z->close(), file_exists("$d/bad.zip"));
var_dump($z->getStatusString(), $z->status);
"#,
    );
}

#[test]
fn lifecycle_matches_php() {
    check(
        "lifecycle",
        r#"
file_put_contents("$d/src.txt", str_repeat("line\n", 50));
// an archive that is never closed is written when the object goes
function make($d) { $z = new ZipArchive; $z->open("$d/a.zip", ZipArchive::CREATE); $z->addFromString('x', 'hello'); }
make($d);
var_dump(file_exists("$d/a.zip"));
$g = new ZipArchive; $g->open("$d/b.zip", ZipArchive::CREATE); $g->addFromString('c:\win.txt', 'w'); $g->addFromString('d/e.txt', 'e');
var_dump(file_exists("$d/b.zip"));
unset($g);
var_dump(file_exists("$d/b.zip"));
$h = new ZipArchive; $h->open("$d/b.zip"); mkdir("$d/ex");
var_dump($h->extractTo("$d/ex"), scandir("$d/ex"), file_get_contents("$d/ex/c:\win.txt"));
$h->close();
$z = new ZipArchive; var_dump($z->open("$d/p.zip", ZipArchive::CREATE));
$z->addFromString('s', str_repeat('a', 300)); $z->addFile("$d/src.txt", 'f');
var_dump($z->statIndex(0)['comp_size'], $z->statIndex(1)['comp_size'], $z->statIndex(1)['size']);
var_dump(strlen($z->getFromIndex(0)), $z->status);
var_dump($z->getFromIndex(1), $z->status, $z->getStatusString());
var_dump($z->getFromName('f'), $z->getStream('f'));
var_dump($z->addFile("$d/nope/x"));
try { $z->addFile("a\0b"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump($z->close());
$r = new ZipArchive; $r->open("$d/p.zip");
var_dump($r->statIndex(1)['comp_size'], strlen($r->getFromName('f')));
$r->close();
try { $r->open("a\0b"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { $r->close(); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { clone $r; } catch (Error $e) { echo $e->getMessage(), "\n"; }
"#,
    );
}

#[test]
fn properties_and_views_match_php() {
    check(
        "props",
        r#"
class Holder extends ZipArchive { public $extra = 1; protected $p = 2; }
$z = new Holder;
var_dump($z);
var_dump($z->open("$d/a.zip", ZipArchive::CREATE), $z->addFromString('x', 'y'));
print_r($z); echo "\n";
var_dump((array) $z, get_object_vars($z), json_encode($z));
var_export($z); echo "\n";
foreach (['lastId', 'status', 'statusSys', 'numFiles', 'comment', 'bogus'] as $p) {
    var_dump(isset($z->$p), empty($z->$p));
    try { $z->$p = 5; } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
}
var_dump($z->bogus, array_column([$z, new ZipArchive], 'numFiles'), array_column([$z], 'extra', 'lastId'));
$z->dyn = 3;
var_dump($z->dyn, count($z));
$z->close();
"#,
    );
}

#[test]
fn encryption_and_methods_match_php() {
    check(
        "enc",
        r#"
$f = "$d/e.zip";
foreach ([ZipArchive::CM_STORE, ZipArchive::CM_DEFLATE, ZipArchive::CM_BZIP2, ZipArchive::CM_XZ, 93, 14, 98, 9999] as $m) {
    var_dump(ZipArchive::isCompressionMethodSupported($m), ZipArchive::isCompressionMethodSupported($m, false));
}
foreach ([0, 1, 257, 258, 259, 9999] as $m) {
    var_dump(ZipArchive::isEncryptionMethodSupported($m), ZipArchive::isEncryptionMethodSupported($m, false));
}
$z = new ZipArchive;
$z->open($f, ZipArchive::CREATE);
$z->addFromString('clear.txt', 'clear');
foreach (['aes.txt' => ZipArchive::EM_AES_256, 'aes128.txt' => ZipArchive::EM_AES_128, 'trad.txt' => ZipArchive::EM_TRAD_PKWARE] as $n => $m) {
    $z->addFromString($n, "secret $n");
    var_dump($z->setEncryptionName($n, $m, 'pw'));
}
var_dump($z->setEncryptionName('clear.txt', 9999, 'pw'), $z->setEncryptionIndex(99, ZipArchive::EM_AES_256, 'pw'));
var_dump($z->addFromString('bz.txt', str_repeat('bz', 100)), $z->setCompressionName('bz.txt', ZipArchive::CM_BZIP2));
var_dump($z->close());
$z->open($f);
foreach (['clear.txt', 'aes.txt', 'aes128.txt', 'trad.txt', 'bz.txt'] as $n) {
    $s = $z->statName($n);
    var_dump($s['encryption_method'], $s['comp_method'], $z->getFromName($n), $z->status);
}
var_dump($z->setPassword(''), $z->setPassword('wrong'), $z->getFromName('aes.txt'), $z->status, $z->getStatusString());
var_dump($z->setPassword('pw'), $z->getFromName('aes.txt'), $z->getFromName('trad.txt'));
$z->close();
var_dump(file_get_contents("zip://$f#aes.txt"));
var_dump(file_get_contents("zip://$f#aes.txt", false, stream_context_create(['zip' => ['password' => 'pw']])));
var_dump(file_get_contents("zip://$f#aes.txt", false, stream_context_create(['zip' => ['password' => 42]])));
"#,
    );
}

#[test]
fn streams_match_php() {
    check(
        "streams",
        r#"
$f = "$d/s.zip";
$z = new ZipArchive;
$z->open($f, ZipArchive::CREATE);
$z->addFromString('stored', 'stored entry data');
$z->setCompressionName('stored', ZipArchive::CM_STORE);
$z->addFromString('deflated', str_repeat("deflated line\n", 40));
$z->close();
$z->open($f);
foreach (['stored', 'deflated'] as $n) {
    $s = $z->getStream($n);
    $m = stream_get_meta_data($s);
    var_dump($m);
    var_dump(fread($s, 4), ftell($s), fseek($s, 2, SEEK_CUR), fgets($s), ftell($s));
    var_dump(fseek($s, 1, SEEK_SET), fread($s, 3), feof($s), strlen(stream_get_contents($s)), feof($s));
    var_dump(fstat($s), fwrite($s, 'x'));
    fclose($s);
}
var_dump($z->getStream('nope'), $z->getStreamIndex(0, ZipArchive::FL_UNCHANGED) !== false, $z->getStreamName('stored'));
$keep = $z->getStream('deflated');
$z->close();
var_dump(stream_get_contents($keep));
fclose($keep);
$s = fopen("zip://$f#stored", 'rb');
$m = stream_get_meta_data($s);
$m['uri'] = basename($m['uri']);
var_dump($m, fstat($s)['size'], fread($s, 100));
fclose($s);
var_dump(@fopen("zip://$f#nope", 'r'), @fopen("zip://$f#stored", 'w'), @fopen("zip://$f", 'r'), file_exists("zip://$f#stored"));
var_dump(file_get_contents("zip://$f#deflated", false, null, 5, 20), file("zip://$f#stored"));
"#,
    );
}

#[test]
fn callbacks_match_php() {
    check(
        "callbacks",
        r#"
$f = "$d/c.zip";
$z = new ZipArchive;
$z->open($f, ZipArchive::CREATE);
var_dump($z->registerProgressCallback(0.5, function ($r) { echo "progress ", $r, "\n"; }));
$z->addFromString('a', str_repeat('a', 1000));
var_dump($z->close());
$z->open($f, ZipArchive::CREATE);
var_dump($z->registerCancelCallback(function () { echo "cancel?\n"; return -1; }));
$z->addFromString('b', 'b');
var_dump($z->close(), $z->status, $z->getStatusString());
$z->open($f, ZipArchive::CREATE);
$z->registerCancelCallback(function () { throw new Exception('boo'); });
$z->addFromString('c', 'c');
try { var_dump($z->close()); } catch (Exception $e) { echo 'caught ', $e->getMessage(), "\n"; }
$z->open($f, ZipArchive::CREATE);
$z->registerCancelCallback(fn () => '7');
$z->addFromString('d', 'd');
var_dump($z->close(), $z->status);
try { $z->registerProgressCallback(1.0, 'no_such_function'); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
$z->open($f);
try { $z->registerProgressCallback(1.0, 'no_such_function'); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
$z->close();
// freed with a callback and pending changes: closed when the object goes
function freed($f) { $z = new ZipArchive; $z->open($f, ZipArchive::CREATE); $z->registerProgressCallback(1.0, function ($r) { echo "late $r\n"; }); $z->addFromString('e', 'e'); }
freed($f);
echo "after\n";
"#,
    );
}

#[test]
fn stream_edges_match_php() {
    check(
        "stream_edges",
        r#"
$f = "$d/e.zip";
$z = new ZipArchive;
$z->open($f, ZipArchive::CREATE);
$z->addFromString('stored', '0123456789');
$z->setCompressionName('stored', ZipArchive::CM_STORE);
$z->addFromString('big', str_repeat("0123456789abcdef", 10000));
$z->addFromString('secret', "a
b
c
");
$z->setEncryptionName('secret', ZipArchive::EM_AES_256, 'pw');
$z->close();
$z->open($f);
$s = $z->getStream('stored');
var_dump(fread($s, 3), fseek($s, -1), ftell($s), fread($s, 2));
var_dump(fseek($s, 100), ftell($s));
fclose($s);
$s = $z->getStream('big');
var_dump(strlen(stream_get_contents($s, 100000)), strlen(stream_get_contents($s, 20000, 150000)));
var_dump(stream_get_contents($s, 5, 10));
var_dump(ftruncate($s, 0));
$o = fopen('php://memory', 'w+');
var_dump(stream_copy_to_stream($s, $o, 10, 3));
fclose($s);
$ctx = stream_context_create(['zip' => ['password' => 'pw']]);
var_dump(file("zip://$f#secret", 0, $ctx), copy("zip://$f#secret", "$d/out.txt", $ctx), file_get_contents("$d/out.txt"));
var_dump(readfile("zip://$f#secret", false, $ctx));
var_dump(file("zip://$f#nope"));
"#,
    );
}

#[test]
fn implicit_close_matches_php() {
    check(
        "implicit_close",
        r#"
$f = "$d/i.zip";
$z = new ZipArchive;
var_dump($z->addFile("$d/missing.txt"), $z->replaceFile(__FILE__, 0));
// a progress callback's exception comes out of the fclose that closes
$z->open($f, ZipArchive::CREATE);
$z->addFromString('a', 'a');
$z->registerProgressCallback(0.5, function ($r) { throw new Exception("progress $r"); });
$s = $z->getStream('a');
unset($z);
try { var_dump(fclose($s)); } catch (Exception $e) { echo 'caught ', $e->getMessage(), "
"; }
// a failed close names php's active function
function mk($f) {
    $z = new ZipArchive;
    $z->open($f, ZipArchive::CREATE);
    $z->addFromString('b', 'b');
    $z->registerCancelCallback(fn () => -1);
    return $z;
}
$z = mk($f);
unset($z);
class K { function m($f) { $z = mk($f); $z = null; echo "in m
"; } }
(new K)->m($f);
// a subclass's __destruct runs before the close and its callbacks
class MyZip extends ZipArchive {
    function __destruct() { echo "destruct numFiles=", $this->numFiles, "
"; }
}
$z = new MyZip;
$z->open($f, ZipArchive::CREATE);
$z->addFromString('c', 'c');
$z->registerProgressCallback(0.5, function ($r) { echo "progress $r
"; });
unset($z);
echo "end
";
"#,
    );
}

#[test]
fn procedural_api_matches_php() {
    check(
        "procedural",
        r#"
$f = "$d/p.zip";
$z = new ZipArchive;
$z->open($f, ZipArchive::CREATE);
$z->addFromString('one.txt', 'first entry');
$z->addFromString('two.txt', str_repeat('2', 3000));
$z->setCompressionName('one.txt', ZipArchive::CM_STORE);
$z->close();
$r = zip_open($f);
var_dump($r, get_resource_type($r));
while ($e = zip_read($r)) {
    var_dump(get_resource_type($e), zip_entry_name($e), zip_entry_filesize($e), zip_entry_compressedsize($e), zip_entry_compressionmethod($e));
    var_dump(zip_entry_open($r, $e), zip_entry_read($e, 5), strlen(zip_entry_read($e)), zip_entry_read($e, 0));
    var_dump(zip_entry_close($e));
    try { zip_entry_name($e); } catch (TypeError $t) { echo $t->getMessage(), "\n"; }
}
var_dump(zip_read($r));
zip_close($r);
try { zip_read($r); } catch (TypeError $t) { echo $t->getMessage(), "\n"; }
var_dump(zip_open("$d/missing.zip"), zip_open($d));
try { zip_open(''); } catch (ValueError $t) { echo $t->getMessage(), "\n"; }
try { zip_read(fopen('php://memory', 'r')); } catch (TypeError $t) { echo $t->getMessage(), "\n"; }
try { zip_read('x'); } catch (TypeError $t) { echo $t->getMessage(), "\n"; }
"#,
    );
}

#[test]
fn constants_and_info_match_php() {
    check(
        "consts",
        r#"
$c = (new ReflectionClass('ZipArchive'))->getConstants();
foreach ($c as $k => $v) { echo "$k=", var_export($v, true), "\n"; }
var_dump(ZipArchive::LIBZIP_VERSION === phpversion('zip') ? 'same' : 'differs');
if (defined('ZipArchive::FL_RECOMPRESS')) { var_dump(ZipArchive::FL_RECOMPRESS); }
ob_start();
phpinfo(INFO_MODULES);
$info = ob_get_clean();
preg_match('/^zip\n\n(.*?)\n\n/sm', $info, $m);
echo $m[1], "\n";
var_dump(in_array('zip', stream_get_wrappers(), true));
"#,
    );
}
