<?php
class A implements JsonSerializable { function __construct(public $v) {} function jsonSerialize(): mixed { return ['v' => $this->v, 'k' => 'x']; } }
class B implements JsonSerializable { public $p = 1; function jsonSerialize(): mixed { return $this; } }
class C implements JsonSerializable { function jsonSerialize(): mixed { return [$this]; } }
class D implements JsonSerializable { function jsonSerialize(): mixed { return new A(new A(3)); } }
echo json_encode(new A(5)), "\n";
echo json_encode([new A([1,2]), 'b' => new B], JSON_PRETTY_PRINT), "\n";
var_dump(json_encode(new C)); echo json_last_error(), " ", json_last_error_msg(), "\n";
echo json_encode(new D), "\n";
enum E: string { case X = "x"; } enum F: int { case One = 1; } enum G { case Z; }
echo json_encode([E::X, F::One, 'k' => E::X]), "\n";
var_dump(json_encode(G::Z)); echo json_last_error(), " ", json_last_error_msg(), "\n";
try { json_encode(G::Z, JSON_THROW_ON_ERROR); } catch (JsonException $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
class Ser implements JsonSerializable { function jsonSerialize(): mixed { throw new Exception("js"); } }
try { var_dump(json_encode(new Ser)); } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
try { var_dump(json_encode([1, [new Ser]], JSON_PARTIAL_OUTPUT_ON_ERROR)); } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
echo json_last_error(), "\n";
