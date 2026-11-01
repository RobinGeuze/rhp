<?php
$a=['d'=>1,'b'=>2,'a'=>3,'c'=>4]; $b=['c'=>0,'a'=>0,'x'=>0];
var_dump(array_intersect_ukey($a,$b, fn($x,$y)=>$x==$y?0:1));
var_dump(array_uintersect_uassoc(['a'=>'x','b'=>'y'], ['B'=>'Y','a'=>'X'], 'strcasecmp', fn($x,$y)=>strcasecmp($x,$y)?1:0));
$log = function($x,$y){ echo "$x-$y "; return $x <=> $y; };
var_dump(array_intersect_ukey(['a'=>1,'b'=>2,'c'=>3], ['c'=>1,'a'=>2], $log)); echo "\n";
var_dump(array_diff_uassoc([1,2], [1,2], $log)); echo "\n";
var_dump(array_udiff([5,3,1,4,2,9,8,7,6,10,15,12,11,14,13,16,20,19,18,17], [3,6,9,12,15,18], $log)); echo "\n";
var_dump(array_uintersect([5,3,1,4,2,3,9], [3,6,9,1], [9,3,2], $log)); echo "\n";
var_dump(array_diff_ukey(['x'=>1,'y'=>2,'z'=>3,'w'=>4], ['y'=>0], ['w'=>0], $log)); echo "\n";
var_dump(array_udiff_uassoc(['a'=>1,'b'=>2,'c'=>3], ['a'=>'1','b'=>5], $log, $log)); echo "\n";
var_dump(array_intersect_uassoc(['a'=>1,'b'=>2,'c'=>3], ['a'=>'1','b'=>5,'c'=>3], ['c'=>3,'a'=>1], $log)); echo "\n";
var_dump(array_udiff_assoc(['a'=>1,'b'=>2,'c'=>3], ['a'=>1], ['b'=>2,'a'=>9], $log)); echo "\n";
var_dump(array_uintersect_assoc(['a'=>1,'b'=>2,'c'=>3], ['a'=>1,'c'=>3], ['b'=>2,'a'=>1], $log)); echo "\n";
var_dump(array_udiff([1,2,3], fn($a,$b)=>$a<=>$b));
var_dump(@array_udiff([3,1,2], [2], fn($a,$b)=>$a>$b));
var_dump(@array_uintersect([3,1,2], [2,1], fn($a,$b)=>$a>$b));
try { array_udiff([1,2,3],[2], function($a,$b){ throw new Exception("boom"); }); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
try { array_udiff([1],[2], 'nope'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { array_intersect_ukey([1], 5, 'strcmp'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
foreach (['array_udiff','array_uintersect','array_diff_ukey','array_intersect_ukey','array_diff_uassoc','array_udiff_assoc','array_uintersect_assoc'] as $f) { try { $f(5, [1], 'strcmp'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; } }
try { array_udiff_uassoc([1], [2], 'nope', 'strcmp'); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$big = array_map(fn($i)=>($i*37)%41, range(1,40));
$n=0; $r = array_udiff($big, [5,10,15], function($a,$b) use (&$n){ $n++; return $a<=>$b; }); echo $n, " ", count($r), "\n";
$big = array_map(fn($i)=>($i*37)%1201, range(1,1200)); $n=0; $r = array_uintersect($big, range(0,1200,7), function($a,$b) use (&$n){ $n++; return $a<=>$b; }); echo $n, " ", count($r), "
";
