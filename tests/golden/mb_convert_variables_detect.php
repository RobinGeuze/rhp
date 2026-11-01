<?php
// mb_convert_variables: non-strict candidate scoring (mb_guess_encoding_for_strings),
// cycle detection (php warns "Cannot handle recursive references" — @-silenced
// here, the sink is placeholder #32), deep nesting, reference/alias semantics.
$s="h\xe9llo"; var_dump(mb_convert_variables('UTF-8','auto',$s),$s);
$s="h\xe9llo"; var_dump(mb_convert_variables('UTF-8','ASCII,UTF-8',$s),$s);
$s="h\xe9llo"; var_dump(mb_convert_variables('UTF-8','UTF-8,ASCII',$s),bin2hex($s));
$s="h\xe9llo"; var_dump(mb_convert_variables('UTF-8','UTF-8,ISO-8859-1',$s),bin2hex($s));
$s="h\xc3\xa9llo"; var_dump(mb_convert_variables('ISO-8859-1','ASCII,UTF-8,ISO-8859-1',$s),bin2hex($s));
$s="\xef\xbb\xbfabc"; $t=["x", ["\xe9t\xe9", "plain"]]; var_dump(mb_convert_variables('UTF-8','ASCII,ISO-8859-1,UTF-8',$s,$t),bin2hex($s), bin2hex($t[1][0]));
$e=""; var_dump(mb_convert_variables('UTF-8','ASCII,UTF-8',$e));
$r=[1]; $r[]=&$r; var_dump(@mb_convert_variables('UTF-8','ISO-8859-1',$r));
$r2=["\xe9"]; $r2[]=&$r2; var_dump(@mb_convert_variables('UTF-8','ISO-8859-1,UTF-8',$r2), bin2hex($r2[0]));
$r3=["\xe9"]; $r3[]=&$r3; var_dump(@mb_convert_variables('UTF-8','ISO-8859-1',$r3), bin2hex($r3[0]));
$o=new stdClass; $o->a="\xe9"; $o->self=$o; var_dump(@mb_convert_variables('UTF-8','ISO-8859-1',$o), bin2hex($o->a));
$d="\xe9"; for($i=0;$i<300;$i++) $d=[$d]; var_dump(mb_convert_variables('UTF-8','ISO-8859-1',$d));
$x="\xe9"; $a=[&$x,&$x]; var_dump(mb_convert_variables("UTF-8","ISO-8859-1",$a)); var_dump(bin2hex($x), bin2hex($a[0]), bin2hex($a[1]));
$o=new stdClass; $o->p="\xe9"; $b=[$o,$o]; mb_convert_variables("UTF-8","ISO-8859-1",$b); var_dump(bin2hex($o->p));
$y="\xe9"; mb_convert_variables("UTF-8","ISO-8859-1",$y,$y); var_dump(bin2hex($y));
$in=["\xe9"]; $w=[&$in]; mb_convert_variables("UTF-8","ISO-8859-1",$w); var_dump(bin2hex($in[0]));
$c1=["\xe9"]; $c2=$c1; mb_convert_variables("UTF-8","ISO-8859-1",$c1); var_dump(bin2hex($c1[0]), bin2hex($c2[0]));
$z = "\xe9"; $o2=new stdClass; $o2->r=&$z; mb_convert_variables("UTF-8","ISO-8859-1",$o2); var_dump(bin2hex($z), bin2hex($o2->r));
var_dump(bin2hex(mb_convert_encoding("a\xe9","UTF-8","ASCII")), bin2hex(mb_strtoupper("a\xe9","ASCII")));
// scoring over many mixed candidate lists (deterministic LCG)
$S=7; function rnd($a,$b){global $S; $S=($S*1103515245+12345)%2147483648; return $a+intdiv($S,65536)%($b-$a+1);}
$lists = ['ASCII,UTF-8','UTF-8,ASCII','UTF-8,ISO-8859-1','ISO-8859-1,UTF-8','ASCII,UTF-8,ISO-8859-1','ISO-8859-1,ASCII,UTF-8','auto',['UTF-8','ISO-8859-1','ASCII']];
$pool = ["a","b"," ","!","\xe9","\xc3\xa9","\xe2\x82\xac","\xf0\x9f\x98\x80","\xff","\xc3","\x80","\xef\xbb\xbf","\xe4\xb8\xad","Z","\xa0","-"];
for ($t = 0; $t < 150; $t++) {
    $n = rnd(1, 3); $vars = [];
    for ($k = 0; $k < $n; $k++) { $s = ''; $m = rnd(0, 8); for ($j = 0; $j < $m; $j++) $s .= $pool[rnd(0, count($pool)-1)]; $vars[] = $s; }
    $l = $lists[rnd(0, count($lists)-1)];
    $v = $vars;
    $r = mb_convert_variables('UTF-8', $l, $v);
    echo $t, ' ', $r, ' ', bin2hex(implode('|', $v)), "\n";
}
