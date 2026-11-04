<?php
class T { public $v='pub'; function __toString(): string { return $this->v; } }
$t=new T;
var_dump(strlen($t), strtoupper($t), str_contains($t,'u'), explode('u',$t), substr($t,1), implode(',',[$t]), sprintf('%s',$t), str_repeat($t,2), str_pad($t,5,'-'), strcmp($t,'pub'));
var_dump(str_replace('u', 'U', $t), vsprintf('%s-%s', [$t, $t]), join('+', [$t, 1]));
try { strlen(new stdClass); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { str_replace('a', 'b', new stdClass); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { mb_substr('abc', 0, 1, new stdClass); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { strlen(function() {}); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
var_dump(array_map('strtoupper', [$t]));
var_dump(unserialize(new class { function __toString(): string { return 'i:5;'; } }));
var_dump(in_array($t, ['pub']), $t == 'pub');
class T2{function __construct(public $v){} function __toString():string{return $this->v;}}
class M{static function s(string ...$xs){return $xs;}}
var_dump(M::s(new T2('a'), new T2('b')));
function f(int $a, string ...$r) { return $r; }
var_dump(f(1, new T2('z')));
