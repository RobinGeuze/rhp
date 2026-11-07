<?php
// A user stream closes (stream_close, then the object's destructor) as
// its last reference goes, as php frees a resource.
class ReleaseWrapper {
    public $context;
    private $p = 0;
    private $name;
    function stream_open($path, $mode, $options, &$opened) { $this->name = $path; return true; }
    function stream_read($n) { return $this->p++ < 2 ? "x" : ''; }
    function stream_eof() { return $this->p >= 2; }
    function stream_close() { echo "close {$this->name}\n"; }
    function __destruct() { echo "destruct {$this->name}\n"; }
}
stream_wrapper_register('rel', 'ReleaseWrapper');
function f() { $f = fopen('rel://local', 'r'); fread($f, 1); echo "leaving f\n"; }
f();
echo "after f\n";
$g = fopen('rel://nulled', 'r');
$g = null;
echo "after null\n";
$a = [fopen('rel://in-array', 'r')];
unset($a);
echo "after unset\n";
$h = fopen('rel://kept', 'r');
echo "end\n";
