<?php
// finfo over php's bundled libmagic and magic database.
$samples = [
    'empty' => '',
    'one byte' => 'x',
    'text' => "hello world\n",
    'crlf text' => "line one\r\nline two\r\n",
    'utf-8' => "h\xc3\xa9llo w\xc3\xb6rld\n",
    'latin-1' => "h\xe9llo\n",
    'php' => "<?php echo 1;\n",
    'html' => "<!DOCTYPE html>\n<html><body>x</body></html>\n",
    'xml' => "<?xml version=\"1.0\"?>\n<root/>\n",
    'json' => "{\"a\": [1, 2, {\"b\": null}]}\n",
    'csv' => "a,b,c\n1,2,3\n4,5,6\n",
    'shell' => "#!/bin/sh\necho hi\n",
    'png' => "\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x10\0\0\0\x20\x08\x06\0\0\0",
    'gif' => "GIF89a\x10\0\x20\0\x80\0\0",
    'jpeg' => "\xff\xd8\xff\xe0\0\x10JFIF\0\x01\x01\0\0\x01\0\x01\0\0",
    'pdf' => "%PDF-1.7\n%\xe2\xe3\xcf\xd3\n",
    'gzip' => "\x1f\x8b\x08\0\0\0\0\0\0\x03\xcbH\xcd\xc9\xc9\x07\0",
    'zip' => "PK\x03\x04\x14\0\0\0\x08\0",
    'elf' => "\x7fELF\x02\x01\x01\0" . str_repeat("\0", 8) . "\x02\0\x3e\0\x01\0\0\0",
    'binary' => "\x00\x01\x02\x03\xfe\xff\x00\x10",
];
$finfo = new finfo();
foreach ([FILEINFO_NONE, FILEINFO_MIME_TYPE, FILEINFO_MIME, FILEINFO_MIME_ENCODING, FILEINFO_EXTENSION, FILEINFO_APPLE] as $flags) {
    echo "== flags $flags\n";
    foreach ($samples as $name => $data) {
        echo "$name: ";
        var_dump($finfo->buffer($data, $flags));
    }
}

// The procedural API and set_flags.
$f = finfo_open(FILEINFO_MIME_TYPE);
var_dump(get_class($f), finfo_buffer($f, $samples['png']));
var_dump(finfo_set_flags($f, FILEINFO_MIME));
var_dump(finfo_buffer($f, $samples['text']));
var_dump($finfo->set_flags(FILEINFO_MIME_TYPE), $finfo->buffer($samples['gif']));
var_dump(finfo_buffer($f, $samples['text'], FILEINFO_NONE), finfo_buffer($f, $samples['text']));

// Files, directories and streams.
$tmp = sys_get_temp_dir() . '/rhp-golden-fileinfo-' . getmypid();
@mkdir($tmp);
file_put_contents("$tmp/a.png", $samples['png']);
file_put_contents("$tmp/b.txt", $samples['text']);
var_dump(finfo_file($f, "$tmp/a.png"), $finfo->file("$tmp/b.txt"), $finfo->file($tmp));
var_dump(mime_content_type("$tmp/a.png"), mime_content_type($tmp));
$fp = fopen("$tmp/b.txt", 'r');
fseek($fp, 3);
var_dump(mime_content_type($fp), ftell($fp));
fclose($fp);
$mem = fopen('php://memory', 'w+');
fwrite($mem, $samples['pdf']);
var_dump(mime_content_type($mem));
var_dump(mime_content_type('data://text/plain;base64,' . base64_encode($samples['gif'])));

// A user stream wrapper is read through php's streams.
class MemWrapper {
    public static array $files = [];
    private string $data = '';
    private int $pos = 0;
    public $context;
    function stream_open($path, $mode, $options, &$opened) {
        $this->data = self::$files[substr($path, 6)] ?? '';
        return isset(self::$files[substr($path, 6)]);
    }
    function stream_read($n) { $r = substr($this->data, $this->pos, $n); $this->pos += strlen($r); return $r; }
    function stream_eof() { return $this->pos >= strlen($this->data); }
    function stream_stat() { return ['mode' => 0100644, 'size' => strlen($this->data)]; }
    function stream_cast($as) { return false; }
}
stream_wrapper_register('mem', MemWrapper::class);
MemWrapper::$files['x'] = $samples['jpeg'];
var_dump($finfo->file('mem://x'), mime_content_type('mem://x'));

// A magic file of our own.
file_put_contents("$tmp/my.magic", "0\tstring\tRHPMAGIC\tthe rhp test format, version %s\n>8\tstring\tx\t\\b%s\n!:mime\tapplication/x-rhp-test\n");
$my = new finfo(FILEINFO_NONE, "$tmp/my.magic");
var_dump($my->buffer("RHPMAGIC 1.2"), $my->buffer("RHPMAGIC"), $my->buffer("nothing"));
var_dump($my->buffer("RHPMAGIC 3", FILEINFO_MIME_TYPE));

// Errors.
function show(callable $c) {
    try {
        var_dump($c());
    } catch (Throwable $e) {
        echo get_class($e), ': ', str_replace($GLOBALS['tmp'], 'TMP', $e->getMessage()), "\n";
    }
}
show(fn() => finfo_file($f, ''));
show(fn() => $finfo->file(''));
show(fn() => finfo_file($f, "a\0b"));
show(fn() => new finfo(FILEINFO_NONE, "$tmp/missing"));
show(fn() => new finfo(FILEINFO_NONE, "a\0b"));
show(fn() => mime_content_type(''));
show(fn() => mime_content_type("a\0b"));
show(fn() => mime_content_type([]));
show(fn() => clone $finfo);
show(fn() => serialize($finfo));
show(fn() => (new ReflectionClass('finfo'))->newInstanceWithoutConstructor()->buffer('x'));
show(fn() => finfo_buffer($f, 'x', 0, 'nope'));
set_error_handler(function ($no, $msg) { echo "error $no: ", str_replace($GLOBALS['tmp'], 'TMP', $msg), "\n"; return true; });
show(fn() => finfo_open(FILEINFO_NONE, "$tmp/missing"));
show(fn() => $finfo->file("$tmp/missing"));
show(fn() => finfo_buffer($f, 'x', 0, null));
show(fn() => finfo_close($f));
restore_error_handler();

foreach (glob("$tmp/*") as $p) unlink($p);
rmdir($tmp);
var_dump(FILEINFO_NONE, FILEINFO_SYMLINK, FILEINFO_MIME, FILEINFO_MIME_TYPE, FILEINFO_MIME_ENCODING,
    FILEINFO_DEVICES, FILEINFO_CONTINUE, FILEINFO_PRESERVE_ATIME, FILEINFO_RAW, FILEINFO_APPLE, FILEINFO_EXTENSION);
var_dump(extension_loaded('fileinfo'), function_exists('finfo_open'));
