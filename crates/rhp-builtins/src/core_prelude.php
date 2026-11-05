interface Stringable { public function __toString(): string; }
interface Throwable extends Stringable {
    public function getMessage(): string;
    public function getCode();
    public function getFile(): string;
    public function getLine(): int;
    public function getTrace(): array;
    public function getPrevious(): ?Throwable;
    public function getTraceAsString(): string;
}
interface UnitEnum {}
interface BackedEnum extends UnitEnum {}
interface Traversable {}
interface Iterator extends Traversable {
    public function current(): mixed;
    public function next(): void;
    public function key(): mixed;
    public function valid(): bool;
    public function rewind(): void;
}
interface IteratorAggregate extends Traversable { public function getIterator(): Traversable; }
interface OuterIterator extends Iterator { public function getInnerIterator(): ?Iterator; }
interface RecursiveIterator extends Iterator {
    public function hasChildren(): bool;
    public function getChildren(): ?RecursiveIterator;
}
interface JsonSerializable { public function jsonSerialize(): mixed; }
interface Serializable {
    public function serialize();
    public function unserialize(string $data);
}
interface ArrayAccess {
    public function offsetExists(mixed $offset): bool;
    public function offsetGet(mixed $offset): mixed;
    public function offsetSet(mixed $offset, mixed $value): void;
    public function offsetUnset(mixed $offset): void;
}
interface Countable { public function count(): int; }
final class Generator implements Iterator {}
class stdClass {}
final class __PHP_Incomplete_Class {}
final class Closure {
    public static function fromCallable($callback) { return __closure_from_callable($callback); }
    public static function bind($closure, $newThis, $newScope = 'static') { return __closure_bind($closure, $newThis, $newScope); }
    public static function getCurrent(): Closure { return __closure_current(); }
}
class Exception implements Throwable {
    protected $message = "";
    private $string = "";
    protected $code = 0;
    protected $file = "";
    protected $line = 0;
    private $trace = [];
    private $previous = null;
    public function __construct($message = "", $code = 0, $previous = null) {
        $n = func_num_args();
        if ($n >= 1) { $this->message = __zpp_arg("Exception::__construct", 1, "message", "string", $message); }
        if ($n >= 2) { $c = __zpp_arg("Exception::__construct", 2, "code", "int", $code); if ($c) { $this->code = $c; } }
        if ($n >= 3) { $p = __zpp_arg("Exception::__construct", 3, "previous", "?Throwable", $previous); if ($p !== null) { $this->previous = $p; } }
    }
    final public function getMessage() { return $this->message; }
    final public function getCode() { return $this->code; }
    final public function getPrevious() { return $this->previous; }
    final public function getFile() { return $this->file; }
    final public function getLine() { return $this->line; }
    final public function getTrace() { return $this->trace; }
    final public function getTraceAsString() { return __exception_trace_string($this->trace); }
    public function __toString() { return __throwable_string($this); }
    // ErrorException's constructor sets the private `previous` (php writes
    // it in Exception's scope).
    protected function __setPrevious($p) { $this->previous = $p; }
}
class Error implements Throwable {
    protected $message = "";
    private $string = "";
    protected $code = 0;
    protected $file = "";
    protected $line = 0;
    private $trace = [];
    private $previous = null;
    public function __construct($message = "", $code = 0, $previous = null) {
        $n = func_num_args();
        if ($n >= 1) { $this->message = __zpp_arg("Error::__construct", 1, "message", "string", $message); }
        if ($n >= 2) { $c = __zpp_arg("Error::__construct", 2, "code", "int", $code); if ($c) { $this->code = $c; } }
        if ($n >= 3) { $p = __zpp_arg("Error::__construct", 3, "previous", "?Throwable", $previous); if ($p !== null) { $this->previous = $p; } }
    }
    final public function getMessage() { return $this->message; }
    final public function getCode() { return $this->code; }
    final public function getPrevious() { return $this->previous; }
    final public function getFile() { return $this->file; }
    final public function getLine() { return $this->line; }
    final public function getTrace() { return $this->trace; }
    final public function getTraceAsString() { return __exception_trace_string($this->trace); }
    public function __toString() { return __throwable_string($this); }
}
class ErrorException extends Exception {
    protected $severity = 1;
    public function __construct($message = "", $code = 0, $severity = 1, $filename = null, $line = null, $previous = null) {
        $n = func_num_args();
        $f = "ErrorException::__construct";
        if ($n >= 1) { $this->message = __zpp_arg($f, 1, "message", "string", $message); }
        if ($n >= 2) { $c = __zpp_arg($f, 2, "code", "int", $code); if ($c) { $this->code = $c; } }
        if ($n >= 3) { $severity = __zpp_arg($f, 3, "severity", "int", $severity); }
        if ($n >= 4) { $filename = __zpp_arg($f, 4, "filename", "?string", $filename); }
        if ($n >= 5) { $line = __zpp_arg($f, 5, "line", "?int", $line); }
        if ($n >= 6) { $p = __zpp_arg($f, 6, "previous", "?Throwable", $previous); if ($p !== null) { $this->__setPrevious($p); } }
        $this->severity = $severity;
        if ($filename !== null) { $this->file = $filename; }
        if ($line !== null) { $this->line = $line; } elseif ($filename !== null) { $this->line = 0; }
    }
    final public function getSeverity() { return $this->severity; }
}
class TypeError extends Error {}
class ValueError extends Error {}
class ArithmeticError extends Error {}
class DivisionByZeroError extends ArithmeticError {}
class ArgumentCountError extends TypeError {}
class UnhandledMatchError extends Error {}
class CompileError extends Error {}
class ParseError extends CompileError {}
class AssertionError extends Error {}
class LogicException extends Exception {}
class InvalidArgumentException extends LogicException {}
class LengthException extends LogicException {}
class DomainException extends LogicException {}
class BadFunctionCallException extends LogicException {}
class BadMethodCallException extends BadFunctionCallException {}
class RuntimeException extends Exception {}
class OutOfBoundsException extends RuntimeException {}
class OutOfRangeException extends LogicException {}
class RangeException extends RuntimeException {}
class OverflowException extends RuntimeException {}
class UnderflowException extends RuntimeException {}
class UnexpectedValueException extends RuntimeException {}
class JsonException extends Exception {}
class RequestParseBodyException extends Exception {}
