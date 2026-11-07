<?php
set_error_handler(function ($no, $msg) {
    echo "[$no] $msg\n";
    return true;
});
// A user stream wrapper (stream_wrapper_register) under the stream
// functions the phpt tests leave out.
class MemWrapper {
    public $context;
    static $files = [];
    private $name;
    private $pos = 0;

    function stream_open($path, $mode, $options, &$opened_path) {
        $this->name = substr($path, 6);
        if (strpbrk($mode, 'wx') !== false) {
            self::$files[$this->name] = '';
        } elseif (!isset(self::$files[$this->name])) {
            if (strpbrk($mode, 'ac') === false) {
                return false;
            }
            self::$files[$this->name] = '';
        }
        if (str_contains($mode, 'a')) {
            $this->pos = strlen(self::$files[$this->name]);
        }
        return true;
    }
    function stream_read($count) {
        echo "read($count)\n";
        $r = substr(self::$files[$this->name], $this->pos, $count);
        $this->pos += strlen($r);
        return $r;
    }
    function stream_write($data) {
        echo "write(", strlen($data), ")\n";
        $f = &self::$files[$this->name];
        $f = substr($f, 0, $this->pos) . $data . substr($f, $this->pos + strlen($data));
        $this->pos += strlen($data);
        return strlen($data);
    }
    function stream_eof() {
        return $this->pos >= strlen(self::$files[$this->name]);
    }
    function stream_tell() {
        return $this->pos;
    }
    function stream_seek($offset, $whence) {
        $len = strlen(self::$files[$this->name]);
        $p = match ($whence) {
            SEEK_SET => $offset,
            SEEK_CUR => $this->pos + $offset,
            SEEK_END => $len + $offset,
        };
        if ($p < 0) {
            return false;
        }
        $this->pos = $p;
        return true;
    }
    function stream_stat() {
        return ['size' => strlen(self::$files[$this->name])];
    }
    function url_stat($path, $flags) {
        $n = substr($path, 6);
        if (!isset(self::$files[$n])) {
            return false;
        }
        return ['mode' => 0100644, 'size' => strlen(self::$files[$n])];
    }
    function stream_close() {
        echo "close\n";
    }
    function stream_flush() {
        return true;
    }
}

var_dump(stream_wrapper_register('mem', 'MemWrapper'));
var_dump(in_array('mem', stream_get_wrappers()));
var_dump(stream_wrapper_register('mem', 'MemWrapper'));

var_dump(file_put_contents('mem://a', "line one\nline two\nline three\n"));
var_dump(file_exists('mem://a'), file_exists('mem://b'), filesize('mem://a'));
var_dump(is_file('mem://a'));
clearstatcache();

$f = fopen('mem://a', 'r+');
var_dump(fgets($f));
var_dump(ftell($f));
var_dump(fread($f, 4));
var_dump(stream_get_contents($f));
var_dump(feof($f));
rewind($f);
var_dump(fgetc($f));
fseek($f, 0, SEEK_END);
var_dump(fwrite($f, "line four\n"));
$m = stream_get_meta_data($f);
var_dump($m['wrapper_type'], $m['stream_type'], $m['mode'], $m['uri'], $m['seekable']);
var_dump(get_class($m['wrapper_data']));
fclose($f);

var_dump(file('mem://a', FILE_IGNORE_NEW_LINES));
var_dump(file_get_contents('mem://a', false, null, 5, 8));
readfile('mem://a');
var_dump(@fopen('mem://missing', 'r'));
$f = fopen('mem://missing', 'r');

$a = fopen('mem://a', 'r');
$b = fopen('mem://copy', 'w');
var_dump(stream_copy_to_stream($a, $b));
fclose($a);
fclose($b);
var_dump(MemWrapper::$files['copy'] === MemWrapper::$files['a']);

var_dump(stream_wrapper_unregister('mem'));
var_dump(@file_get_contents('mem://a'));
var_dump(stream_wrapper_restore('mem'));
