<?php
set_error_handler(function ($no, $msg) {
    echo "[$no] $msg\n";
    return true;
});
// User stream wrappers: directories, url operations, include, exceptions.
class W {
    public $context;
    private $entries;
    private $data;
    private $pos = 0;
    function __construct() {
        echo "construct, context is ", gettype($this->context), "\n";
    }
    function dir_opendir($path, $options) {
        echo "dir_opendir($path, $options)\n";
        $this->entries = ['b', 'a', 'c'];
        return true;
    }
    function dir_readdir() {
        return array_shift($this->entries) ?? false;
    }
    function dir_rewinddir() {
        echo "rewind\n";
        $this->entries = ['b', 'a', 'c'];
        return true;
    }
    function dir_closedir() {
        echo "closedir\n";
    }
    function mkdir($path, $mode, $options) {
        printf("mkdir(%s, %o, %d)\n", $path, $mode, $options);
        return true;
    }
    function rmdir($path, $options) {
        echo "rmdir($path, $options)\n";
        return false;
    }
    function rename($from, $to) {
        echo "rename($from, $to)\n";
        return true;
    }
    function unlink($path) {
        echo "unlink($path)\n";
        return 1;
    }
    function url_stat($path, $flags) {
        echo "url_stat($path, $flags)\n";
        if (str_contains($path, 'boom')) {
            throw new RuntimeException("no stat for $path");
        }
        return ['mode' => 040755];
    }
    function stream_open($path, $mode, $options, &$opened) {
        if (str_contains($path, 'boom')) {
            throw new LogicException('cannot open');
        }
        $this->data = '<?php echo "included from ", __FILE__, "\n"; return 42;';
        $opened = 'w://opened/path';
        return true;
    }
    function stream_read($n) {
        $r = substr($this->data, $this->pos, $n);
        $this->pos += strlen($r);
        return $r;
    }
    function stream_eof() {
        return $this->pos >= strlen($this->data);
    }
    function stream_stat() {
        return [];
    }
    function stream_set_option($o, $a, $b) {
        return false;
    }
}
var_dump(stream_wrapper_register('w', 'W') === false);
var_dump(stream_wrapper_register('ww', 'W'));

$d = opendir('ww://dir');
var_dump(readdir($d), readdir($d));
rewinddir($d);
var_dump(readdir());
closedir($d);
var_dump(scandir('ww://dir'));
var_dump(mkdir('ww://new', 0700, true));
var_dump(rmdir('ww://new'));
var_dump(rename('ww://x', 'ww://y'));
var_dump(@rename('ww://x', '/tmp/y'));
var_dump(unlink('ww://x'));
var_dump(is_dir('ww://x'), is_dir('ww://x'));
try {
    file_exists('ww://boom');
} catch (RuntimeException $e) {
    echo $e->getMessage(), "\n";
}

var_dump(include 'ww://file.php');
try {
    fopen('ww://boom', 'r');
} catch (LogicException $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
    foreach ($e->getTrace() as $f) {
        echo '  ', $f['class'] ?? '', $f['type'] ?? '', $f['function'], "\n";
    }
}
try {
    stat('ww://boom');
} catch (RuntimeException $e) {
    echo $e->getMessage(), "\n";
}
