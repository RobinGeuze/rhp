<?php class A implements JsonSerializable { private $p=1; function jsonSerialize(): mixed { return ['custom'=>true, 'n'=>new B]; } }
class B { public $x = 1; protected $y = 2; }
echo json_encode(new A), "\n";
enum S:string{case A='a';} enum U{case A;} enum I:int{case One=1;}
echo json_encode([S::A, I::One, 'k'=>S::A]), "\n"; var_dump(json_encode(U::A), json_last_error_msg(), json_last_error(), JSON_ERROR_NON_BACKED_ENUM);
var_dump(json_encode([1, U::A], JSON_PARTIAL_OUTPUT_ON_ERROR), json_last_error_msg());
class Me implements JsonSerializable { public $a = 1; function jsonSerialize(): mixed { return $this; } } echo json_encode(new Me), "\n";
class Rec implements JsonSerializable { function jsonSerialize(): mixed { return [$this]; } } var_dump(json_encode(new Rec), json_last_error_msg());
$o=new stdClass; $o->o=$o; var_dump(json_encode($o), json_last_error_msg(), json_encode($o, JSON_PARTIAL_OUTPUT_ON_ERROR), json_last_error());
$a=[1]; $a[]=&$a; var_dump(json_encode($a), json_last_error_msg());
class T implements JsonSerializable { function jsonSerialize(): mixed { throw new RuntimeException('boom'); } }
try { json_encode([new T]); } catch (RuntimeException $e) { echo "caught ", $e->getMessage(), "\n"; }
var_dump(json_encode(['ok']), json_last_error());
echo json_encode(['list' => [new A, S::A]], JSON_PRETTY_PRINT), "\n";
$shared = new B; echo json_encode([$shared, $shared]), "\n";
try { json_encode(U::A, JSON_THROW_ON_ERROR); } catch (JsonException $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
