<?php
// Exception / Error / ErrorException constructor argument coercion and the
// TypeErrors php's zpp raises (named after the declaring internal class).
function rel($s) { return str_replace(__DIR__ . '/', '', $s); }
class Hello extends Exception {}
class MyErr extends ErrorException {}
class Msg { public function __toString(): string { return "from toString"; } }

$cases = [
    fn() => new Hello(new stdClass()),
    fn() => new Exception([]),
    fn() => new Error("m", "abc"),
    fn() => new Exception("m", 1, new stdClass()),
    fn() => new MyErr(new stdClass()),
    fn() => new ErrorException("m", 0, "x"),
    fn() => new ErrorException("m", 0, 1, []),
];
foreach ($cases as $make) {
    try {
        $make();
        echo "no error\n";
    } catch (TypeError $e) {
        echo get_class($e), ": ", $e->getMessage(), " @", $e->getLine(), "\n";
        echo rel($e->getTraceAsString()), "\n";
    }
}

$e = new Exception(42, "7");
var_dump($e->getMessage(), $e->getCode());
$e = new Exception(new Msg());
var_dump($e->getMessage());
$e = new Exception(1.5, true);
var_dump($e->getMessage(), $e->getCode());
$e = new Error("x", 0, new Exception("prev"));
var_dump($e->getPrevious()->getMessage());
$e = new ErrorException("y", "3", "2", null, null, null);
var_dump($e->getCode(), $e->getSeverity(), basename($e->getFile()), $e->getLine());
$e = new ErrorException("y", 0, 1, "f.php", "12");
var_dump($e->getFile(), $e->getLine());
