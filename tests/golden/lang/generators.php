<?php
function counter(int $n) { for ($i = 1; $i <= $n; $i++) yield $i; }
foreach (counter(5) as $v) echo $v;
echo "\n";

function kv() { yield 'a' => 1; yield 'b' => 2; }
foreach (kv() as $k => $v) echo "$k=$v ";
echo "\n";

$g = counter(3);
echo $g->current(); $g->next(); echo $g->current();
var_dump($g->valid());
$g->next(); $g->next();
var_dump($g->valid());
echo "\n";

function withReturn() { yield 1; yield 2; return 99; }
$r = withReturn();
foreach ($r as $v) echo $v;
echo " ret=", $r->getReturn(), "\n";

function naturals() { $i = 0; while (true) yield $i++; }
$sum = 0;
foreach (naturals() as $n) { if ($n >= 5) break; $sum += $n; }
echo $sum, "\n";

function fib() { [$a, $b] = [0, 1]; while (true) { yield $a; [$a, $b] = [$b, $a + $b]; } }
$out = [];
foreach (fib() as $f) { if (count($out) >= 10) break; $out[] = $f; }
echo implode(",", $out), "\n";

$x = counter(1);
var_dump($x instanceof Generator, $x instanceof Traversable, $x instanceof Iterator);

function echoTimes() { while (true) { $m = yield; if ($m === null) return; echo "got:$m "; } }
$e = echoTimes();
$e->current();
$e->send("a");
$e->send("b");
echo "\n";

class Seq {
    public function __construct(private int $lo, private int $hi) {}
    public function each() { for ($i = $this->lo; $i <= $this->hi; $i++) yield $i => $i * $i; }
}
$s = new Seq(2, 4);
foreach ($s->each() as $k => $v) echo "$k->$v ";
echo "\n";
