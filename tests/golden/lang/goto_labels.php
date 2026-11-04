<?php
$i=0; a: $i++; if($i<3) goto a; echo $i; goto b; echo 'no'; b: echo "end\n";
function f(){foreach([1,2,3] as $x){if($x==2) goto out; echo $x;} out: echo "out\n";}
f();
function g() {
  try {
    echo "try\n";
    goto done;
  } finally {
    echo "finally\n";
  }
  echo "skipped\n";
  done:
  echo "done\n";
}
g();
function h($n) {
  switch ($n) {
    case 1: goto one;
    default: goto other;
  }
  one: return "one";
  other: return "other";
}
echo h(1), h(2), "\n";
function w() {
  $k = 0;
  while (true) {
    while (true) {
      if (++$k > 3) goto escape;
    }
  }
  escape:
  return $k;
}
var_dump(w());
function tc() {
  try { throw new Exception('x'); } catch (Exception $e) { goto after; } finally { echo "fin\n"; }
  after: echo "after\n";
  try { goto inner; echo "no\n"; inner: echo "inner\n"; } finally { echo "fin2\n"; }
}
tc();
class P { function m() { $r = []; start: $r[] = count($r); if (count($r) < 3) goto start; return $r; } }
echo implode(',', (new P)->m()), "\n";
$f = function () { goto x; echo "no"; x: return "closure-goto"; };
echo $f(), "\n";
function gen() { $i = 0; loop: yield $i++; if ($i < 3) goto loop; }
foreach (gen() as $v) echo $v; echo "\n";
