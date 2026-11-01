<?php
// json_encode / var_export of enums and objects: backed enums encode as
// their value, pure enums fail, JsonSerializable is honoured, only public
// properties are JSON-encoded; var_export prints \Enum::Case,
// \Class::__set_state(array(...)) and (object) array(...).
class J implements JsonSerializable {
    private $x = 1;
    public $y = 2;
    function jsonSerialize(): mixed { return ["j" => 1, "e" => S::A, "n" => new Nested]; }
}
class Nested implements JsonSerializable { function jsonSerialize(): mixed { return [I::One, I::Two]; } }
class SelfRet implements JsonSerializable {
    private $x = 1;
    public $y = 2;
    function jsonSerialize(): mixed { return $this; }
}
class K { private $x = 1; protected $z = 3; public $y = 2; public $n = null; }
class T implements JsonSerializable { function jsonSerialize(): mixed { throw new RuntimeException("boom"); } }
enum S: string { case A = "a"; case Slash = "a/b"; }
enum I: int { case One = 1; case Two = 2; }
enum U { case X; }

echo json_encode([new J, new K, S::A, I::One, new SelfRet, "k" => (object)[]]), "\n";
echo json_encode([S::Slash, "o" => new K], JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES), "\n";
var_dump(json_encode(U::X), json_last_error(), json_last_error_msg());
var_dump(json_encode([1, [U::X]]), json_last_error_msg());
var_dump(json_encode([1, [U::X]], JSON_PARTIAL_OUTPUT_ON_ERROR) !== null);
try {
    json_encode(U::X, JSON_THROW_ON_ERROR);
} catch (JsonException $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
try {
    json_encode([new T]);
} catch (RuntimeException $e) {
    echo "caught ", $e->getMessage(), "\n";
}
echo json_encode(function () {}), "\n";
var_dump(json_encode(S::A) === '"a"', json_last_error());

class A { protected $r = 1; private $s = [1, [2]]; public $t; public $o; }
$a = new A;
$a->o = (object)["x" => S::A, "y" => [U::X], "n\0m" => "a\0b", 5 => 'five'];
var_export($a);
echo "\n";
var_export([S::A, U::X, I::Two, PHP_INT_MIN, 1.0, -0.0, "it's\\", "nul\0byte"]);
echo "\n";
var_export(S::A);
echo "\n";
var_export(new stdClass);
echo "\n";
var_export([new stdClass, [], function () {}]);
echo "\n";
echo var_export(U::X, true), "|", var_export(1.5, true), "\n";
var_dump(var_export(I::One, true) === '\I::One');
