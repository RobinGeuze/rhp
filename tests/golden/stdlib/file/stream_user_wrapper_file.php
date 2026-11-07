<?php
// A user wrapper replacing file:// that passes through (as Rector and
// PHPUnit do): include, require_once, the stat family.
final class Passthru {
    public $context;
    private $h;
    private static $calls = [];
    static function on() { stream_wrapper_unregister('file'); stream_wrapper_register('file', self::class); }
    static function off() { stream_wrapper_restore('file'); }
    private function run(callable $f) { self::off(); try { return $f(); } finally { self::on(); } }
    function stream_open($path, $mode, $options, &$opened) {
        echo "open $mode ", basename($path), " ", $options, "\n";
        $this->h = $this->run(fn() => fopen($path, $mode));
        if ($this->h) { $opened = $path; }
        return (bool)$this->h;
    }
    function stream_read($n) { return $this->run(fn() => fread($this->h, $n)); }
    function stream_write($d) { return $this->run(fn() => fwrite($this->h, $d)); }
    function stream_eof() { return $this->run(fn() => feof($this->h)); }
    function stream_stat() { return $this->run(fn() => fstat($this->h)); }
    function stream_close() { $this->run(fn() => fclose($this->h)); }
    function stream_set_option($o, $a, $b) { return false; }
    function stream_seek($o, $w) { return $this->run(fn() => fseek($this->h, $o, $w) === 0); }
    function stream_tell() { return $this->run(fn() => ftell($this->h)); }
    function url_stat($path, $flags) {
        return $this->run(fn() => ($flags & STREAM_URL_STAT_QUIET) ? @stat($path) : stat($path));
    }
}
Passthru::on();
var_dump(file_exists(__DIR__ . '/stream_user_wrapper_file.inc'));
var_dump(include __DIR__ . '/stream_user_wrapper_file.inc');
var_dump(require_once 'stream_user_wrapper_file.inc');
var_dump(strlen(file_get_contents(__FILE__)) > 100);
var_dump(is_dir(__DIR__));
Passthru::off();
var_dump(in_array('file', stream_get_wrappers()));
