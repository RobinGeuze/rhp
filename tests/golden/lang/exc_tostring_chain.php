<?php
// Exception::__toString chains, ErrorException, property defaults, traces of
// user-created throwables, and trace argument rendering.
function rel($s) { return str_replace(__DIR__ . '/', '', $s); }

enum Suit: string { case Hearts = 'h'; }
class Thing {}

function thrower($a, $b, $c, $d, $e, $f, $g, $h, $i, ...$more) {
    return new RuntimeException("made", 7, new LogicException("inner cause"));
}
$e = thrower(
    42,
    "a string longer than fifteen bytes",
    "tab\there\n\x01é",
    1.5,
    2.0,
    null,
    true,
    [1, 2],
    new Thing(),
    Suit::Hearts,
    false,
    -0.0
);
echo rel((string)$e), "\n===\n";
echo rel($e->getTraceAsString()), "\n===\n";
var_dump(count($e->getTrace()[0]['args']));
echo $e->getPrevious()->getMessage(), "|", basename($e->getPrevious()->getFile()), "|", $e->getPrevious()->getLine(), "\n";

// Message-less throwables print just the class.
echo rel((string)new Exception()), "\n===\n";
echo rel((string)new Error("", 3)), "\n===\n";

// An Error as the previous of an Exception (and vice versa).
$chain = new Exception("outer", 1, new TypeError("middle", 2, new Exception("root")));
echo rel((string)$chain), "\n===\n";

// Subclass property defaults survive a no-arg constructor.
class Defaults extends Exception {
    protected $message = "default message";
    protected $code = 99;
}
$d = new Defaults();
echo $d->getMessage(), "|", $d->getCode(), "\n";
$d = new Defaults("given");
echo $d->getMessage(), "|", $d->getCode(), "\n";

// A user __toString override is used by string conversion.
class Pretty extends Exception {
    public function __toString(): string { return "Pretty(" . $this->getMessage() . ")"; }
}
echo new Pretty("p"), "\n";

// ErrorException: severity, explicit file/line.
$ee = new ErrorException("ee", 3, E_WARNING, "/some/where/file.php", 77);
echo $ee->getSeverity(), "|", $ee->getFile(), "|", $ee->getLine(), "|", $ee->getCode(), "\n";
$ee = new ErrorException("ee", 0, E_NOTICE, "/some/where/file.php");
echo $ee->getSeverity(), "|", $ee->getFile(), "|", $ee->getLine(), "\n";
$ee = new ErrorException("ee");
echo $ee->getSeverity(), "|", basename($ee->getFile()), "|", $ee->getLine(), "\n";
$ee = new ErrorException("ee", 0, E_USER_ERROR, null, 5);
echo basename($ee->getFile()), "|", $ee->getLine(), "\n";

var_dump($e instanceof Stringable, $e instanceof Throwable, new Error() instanceof Stringable);

// Lines come from the call / `new` expression, not the statement start.
function make() {
    return
        new InvalidArgumentException(
            "multi"
        );
}
echo make()->getLine(), "\n";
try {
    $r = 1 +
        intdiv(
            1,
            0
        );
} catch (DivisionByZeroError $z) {
    echo $z->getLine(), " ", rel($z->getTraceAsString()), "\n";
}

