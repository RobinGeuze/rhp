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
class ClosedGeneratorException extends Exception {}
// Zend/zend_fibers.c: the engine switches stacks behind __fiber (op codes:
// rhp_builtins::zend::fibers::FiberOp).
final class Fiber {
    public function __construct($callback) { __fiber(0, $this, $callback); }
    public function start(mixed ...$args): mixed { return __fiber(1, $this, $args); }
    public function resume(mixed $value = null): mixed { return __fiber(2, $this, $value); }
    public function throw(Throwable $exception): mixed { return __fiber(3, $this, $exception); }
    public function isStarted(): bool { return __fiber(4, $this); }
    public function isSuspended(): bool { return __fiber(5, $this); }
    public function isRunning(): bool { return __fiber(6, $this); }
    public function isTerminated(): bool { return __fiber(7, $this); }
    public function getReturn(): mixed { return __fiber(8, $this); }
    public static function getCurrent(): ?Fiber { return __fiber(9, null); }
    public static function suspend(mixed $value = null): mixed { return __fiber(10, null, $value); }
}
class stdClass {}
final class __PHP_Incomplete_Class {}
final class InternalIterator implements Iterator {
    private function __construct() {}
    public function current(): mixed { return __internal_iterator($this, 'current'); }
    public function key(): mixed { return __internal_iterator($this, 'key'); }
    public function next(): void { __internal_iterator($this, 'next'); }
    public function valid(): bool { return __internal_iterator($this, 'valid'); }
    public function rewind(): void { __internal_iterator($this, 'rewind'); }
}
final class WeakReference {
    public function __construct() { __weakref('construct'); }
    public static function create(object $object): WeakReference { return __weakref('create', $object); }
    public function get(): ?object { return __weakref('get', $this); }
}
final class WeakMap implements ArrayAccess, Countable, IteratorAggregate {
    public function offsetGet($object): mixed { return __weakmap('get', $this, $object); }
    public function offsetSet($object, mixed $value): void { __weakmap('set', $this, $object, $value); }
    public function offsetExists($object): bool { return __weakmap('exists', $this, $object); }
    public function offsetUnset($object): void { __weakmap('unset', $this, $object); }
    public function count(): int { return __weakmap('count', $this); }
    public function getIterator(): Iterator { return __weakmap('iterator', $this); }
}
// Methods in stub order (get_class_methods, getMethods). A closure value
// runs bindTo()/call() natively; their bodies here are never reached.
final class Closure {
    private function __construct() {}
    public static function bind($closure, $newThis, $newScope = 'static') { return __closure_bind($closure, $newThis, $newScope); }
    public function bindTo($newThis, $newScope = 'static') { return __closure_bind($this, $newThis, $newScope); }
    public function call($newThis, ...$args) { return null; }
    public static function fromCallable($callback) { return __closure_from_callable($callback); }
    public static function getCurrent(): Closure { return __closure_current(); }
}
class Exception implements Throwable {
    protected $message = "";
    private string $string = "";
    protected $code = 0;
    protected string $file = "";
    protected int $line = 0;
    private array $trace = [];
    private ?Throwable $previous = null;
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
    public function __toString() { return $this->string = __throwable_string($this); }
    // ErrorException's constructor sets the private `previous` (php writes
    // it in Exception's scope).
    protected function __setPrevious($p) { $this->previous = $p; }
}
class Error implements Throwable {
    protected $message = "";
    private string $string = "";
    protected $code = 0;
    protected string $file = "";
    protected int $line = 0;
    private array $trace = [];
    private ?Throwable $previous = null;
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
    public function __toString() { return $this->string = __throwable_string($this); }
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
final class FiberError extends Error {
    public function __construct() { throw new Error('The "FiberError" class is reserved for internal use and cannot be manually instantiated'); }
}
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
