<?php
chdir(sys_get_temp_dir());
$self = 'rhp_zdiag_plain.txt';
file_put_contents($self, "plain text, not compressed\n");
function last() { $e = error_get_last(); error_clear_last(); return $e === null ? "(none)" : $e['type'] . ': ' . $e['message']; }
// decode failures warn with zError()
var_dump(@gzuncompress('garbage')); echo last(), "\n";
var_dump(@gzinflate('')); echo last(), "\n";
var_dump(@gzuncompress(gzcompress('hello world'), 3)); echo last(), "\n";
var_dump(@zlib_decode('x')); echo last(), "\n";
var_dump(@gzdecode('abc')); echo last(), "\n";
var_dump(@gzdecode(substr(gzencode('hello world'), 0, 12))); echo last(), "\n";
var_dump(gzuncompress(gzcompress('ok'))); echo last(), "\n";
// open failures
var_dump(@gzopen('/nonexistent-dir/x.gz', 'r')); echo last(), "\n";
var_dump(@gzopen('/nonexistent-dir/x.gz', 'w')); echo last(), "\n";
var_dump(@gzopen($self, 'q')); echo last(), "\n";
var_dump(@gzopen($self, 'r+')); echo last(), "\n";
var_dump(@gzfile('/nonexistent-dir/f')); echo last(), "\n";
var_dump(@readgzfile('/nonexistent-dir/f')); echo last(), "\n";
var_dump(@bzopen('/nonexistent-dir/f.bz2', 'r')); echo last(), "\n";
try { bzopen('', 'r'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { bzopen($self, 'rw'); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
// a non-bzip2 file opens; the read fails with DATA_ERROR_MAGIC
$h = bzopen($self, 'r');
var_dump(is_resource($h));
try { bzread($h, -1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(bzread($h, 10), bzerrno($h), bzerrstr($h), bzerror($h));
// a real bzip2 file reads fine
$tmp = sys_get_temp_dir() . '/rhp_bzdiag_' . getmypid();
file_put_contents($tmp . '.bz2', bzcompress('bzip2 payload'));
$b = bzopen($tmp . '.bz2', 'r');
var_dump(bzread($b, 100), bzerrno($b), bzerrstr($b));
bzclose($b);
unlink($tmp . '.bz2');
// compress.zlib:// has no url_stat
file_put_contents("compress.zlib://$tmp.gz", str_repeat('x', 1000));
var_dump(file_exists("compress.zlib://$tmp.gz"), is_file("compress.zlib://$tmp.gz"), file_exists("$tmp.gz"), strlen(file_get_contents("compress.zlib://$tmp.gz")));
$g = gzopen("$tmp.gz", 'r');
var_dump(@gzseek($g, 0, 2)); echo last(), "\n";
gzclose($g);
unlink("$tmp.gz");
unlink($self);
