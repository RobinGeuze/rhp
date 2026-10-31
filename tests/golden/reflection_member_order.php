<?php
// Reflection enumerates members in php's order: own (declaration order), then
// trait, then inherited, then interface members — while the object layout
// (var_dump, get_object_vars) stays parent-first.
interface I { const X = 9; function im(); }
trait T { const TC = 7; function tm() {} }
class P { const A = 1; const B = 2; public $pa; public $pb; function pm() {} function shared() {} }
class C extends P implements I { use T; const Z = 3; const A = 4; public $c; function im() {} function shared() {} function cm() {} }
class G { public $g; const GC = 0; function gm() {} }
class Q extends G { public $q; const QC = 1; function qm() {} }
class R extends Q { public $r; const RC = 2; function rm() {} }
enum Suit { case Hearts; case Spades; case Clubs; }

$names = fn($xs) => array_map(fn($x) => $x->name, $xs);
foreach (['C', 'R'] as $cls) {
    $rc = new ReflectionClass($cls);
    echo $cls, " constants: ", json_encode($rc->getConstants()), "\n";
    echo $cls, " methods: ", json_encode($names($rc->getMethods())), "\n";
    echo $cls, " properties: ", json_encode($names($rc->getProperties())), "\n";
}
echo "Suit cases: ", json_encode(array_keys((new ReflectionClass('Suit'))->getConstants())), "\n";
echo "layout: ", json_encode(array_keys(get_object_vars(new R))), "\n";
print_r(new R);
