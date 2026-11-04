//! The built-in classes and helper functions written in PHP, compiled once per thread.

use std::cell::RefCell;
use std::rc::Rc;

use rhp_bytecode::{ClassProto, FunctionProto};
use rhp_frontend::cst::Statement;
use rhp_values::Thrown;

use crate::decl::{compile_class, compile_interface};
use crate::file::FileCtx;
use crate::functions::compile_function;
use crate::names::NsContext;

/// The built-in exception/throwable class hierarchy, as PHP source. Compiled
/// and prepended to every program so `throw`/`catch` and engine errors work.
const PRELUDE: &str = r##"<?php
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
        if ($n >= 6) { $p = __zpp_arg($f, 6, "previous", "?Throwable", $previous); if ($p !== null) { $this->previous = $p; } }
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
"##;

/// The compiled prelude: built-in classes, helper functions, and the
/// closures their bodies declare (always program closures `0..n`).
pub(crate) struct PreludeParts {
    pub(crate) classes: Vec<ClassProto>,
    pub(crate) functions: Vec<Rc<FunctionProto>>,
    pub(crate) closures: Vec<Rc<FunctionProto>>,
}

thread_local! {
    /// The prelude, compiled on first use and shared by every later compile on
    /// this thread (protos hold `Rc`s, so the cache is per thread — matching
    /// the VM-per-thread model). It depends on no per-file state: it is
    /// compiled in a [`FileCtx`] of its own.
    pub(crate) static PRELUDE_CACHE: RefCell<Option<Rc<PreludeParts>>> = const { RefCell::new(None) };
}

/// The compiled prelude, compiling it on this thread's first call.
pub(crate) fn prelude_classes() -> Result<Rc<PreludeParts>, Thrown> {
    if let Some(p) = PRELUDE_CACHE.with(|c| c.borrow().clone()) {
        return Ok(p);
    }
    // Compiled once per thread, kept for every later request: not counted
    // against a request's memory.
    let parts = rhp_values::mem::uncounted(|| prelude_classes_isolated().map(Rc::new))?;
    PRELUDE_CACHE.with(|c| *c.borrow_mut() = Some(parts.clone()));
    Ok(parts)
}

/// Compile the prelude in a [`FileCtx`] of its own, so the result is
/// independent of whichever file happened to trigger it.
fn prelude_classes_isolated() -> Result<PreludeParts, Thrown> {
    // The engine's core classes (PRELUDE) plus each extension module's own
    // PHP-source classes (GMP, Phar, …) and helper functions (iterator_*, …).
    let src = format!("{PRELUDE}\n{}", rhp_builtins::module_prelude());
    let arena = rhp_frontend::Arena::new();
    let parsed = arena.parse(b"<prelude>", src.as_bytes());
    let flat: Vec<(Rc<NsContext>, &Statement)> = parsed
        .program()
        .statements
        .iter()
        .map(|s| (Rc::new(NsContext::default()), s))
        .collect();
    // Never strict-typed (coercive calls). No file name: the VM treats the
    // protos as internal code (skipped for exception file/line, `[internal
    // function]` in traces). No line map: prelude line numbers are never
    // surfaced. Method calls see the prelude's own by-ref signatures.
    let cx = FileCtx::prelude(&flat);
    let compile = || -> Result<(Vec<ClassProto>, Vec<Rc<FunctionProto>>), Thrown> {
        let mut classes = Vec::new();
        let mut functions = Vec::new();
        for (_, stmt) in &flat {
            match stmt {
                Statement::Class(c) => classes.push(compile_class(&cx, c)?),
                Statement::Interface(i) => classes.push(compile_interface(&cx, i)?),
                Statement::Function(f) => functions.push(Rc::new(compile_function(&cx, f)?)),
                _ => {}
            }
        }
        Ok((classes, functions))
    };
    let r = compile();
    // Closures the prelude declares occupy closure indices `0..n`; a program
    // with the prelude re-seeds them first. Anonymous / nested classes would
    // index per-file tables the cached copy cannot carry.
    assert!(
        cx.anon_classes.borrow().is_empty() && cx.cond_classes.borrow().is_empty(),
        "prelude must not declare anonymous or nested classes"
    );
    assert!(
        cx.take_fatal().is_none(),
        "prelude must compile without errors"
    );
    let (classes, functions) = r?;
    Ok(PreludeParts {
        classes,
        functions,
        closures: cx.closures.take(),
    })
}
