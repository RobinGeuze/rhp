<?php
// Abstract trait method must NOT override an inherited concrete method.
trait Maker {
    abstract public function make(): string;
    public function describe(): string { return "made:" . $this->make(); }
}
abstract class Base {
    public function make(): string { return "base-impl"; }
}
class Widget extends Base {
    use Maker;
}
$w = new Widget();
echo $w->make(), " / ", $w->describe(), "\n";

// Dynamic static method call: $class::$method().
class Ops {
    public static function twice($n) { return $n * 2; }
    public static function neg($n) { return -$n; }
}
$cls = 'Ops';
foreach (['twice', 'neg'] as $m) {
    echo $cls::$m(21), " ";
}
echo "\n";

// By-reference array element.
$log = ['start'];
$wrap = ['entries' => &$log];
$log[] = 'mid';
$wrap['entries'][] = 'end';
echo implode(',', $log), "\n";

// parse_url + pathinfo.
$u = parse_url('https://user@host.example:8080/a/b.json?x=1#f');
echo $u['scheme'], " ", $u['host'], " ", $u['port'], " ", $u['path'], " ", $u['query'], "\n";
$pi = pathinfo('/var/www/app.tar.gz');
echo $pi['dirname'], " ", $pi['basename'], " ", $pi['extension'], " ", $pi['filename'], "\n";

// inet_pton round-trips through inet_ntop.
echo inet_ntop(inet_pton('192.168.1.1')), " ", inet_ntop(inet_pton('::1')), "\n";

// eval/goto refusal is deferred to runtime: a never-executed branch still runs.
function safe() {
    if (false) { eval('return 1;'); }
    return 'ok';
}
echo safe(), "\n";
