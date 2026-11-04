//! The built-in classes and helper functions written in PHP, compiled once per thread.

use std::cell::RefCell;
use std::rc::Rc;

use rhp_bytecode::{ClassProto, FunctionProto};
use rhp_frontend::cst::Statement;
use rhp_values::Thrown;

use crate::decl::{compile_class, compile_interface};
use crate::file::{
    collect_method_refs, ANON_CLASSES, CLOSURES, COND_CLASSES, IN_TRAIT, LINE_STARTS, METHOD_REF,
    SRC_FILE, STRICT_TYPES,
};
use crate::functions::compile_function;
use crate::names::{NsContext, NS_CTX};

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
    /// compiled in an isolated compiler context (see [`prelude_classes`]).
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

/// Compile the prelude with the per-file compiler state (thread-locals) set to
/// a fixed, prelude-only context, restoring the caller's state afterwards, so
/// the result is independent of whichever file happened to trigger it.
fn prelude_classes_isolated() -> Result<PreludeParts, Thrown> {
    // Prelude / built-in classes are never strict-typed (coercive calls).
    let saved_strict = STRICT_TYPES.with(|s| std::mem::replace(&mut *s.borrow_mut(), false));
    // Prelude protos carry an empty file name: the VM treats them as internal
    // code (skipped for exception file/line, `[internal function]` in traces).
    let saved_file = SRC_FILE.with(|f| std::mem::take(&mut *f.borrow_mut()));
    // No line map: prelude line numbers are never surfaced.
    let saved_lines = LINE_STARTS.with(|l| std::mem::take(&mut *l.borrow_mut()));
    let saved_ns =
        NS_CTX.with(|c| std::mem::replace(&mut *c.borrow_mut(), Rc::new(NsContext::default())));
    let saved_trait = IN_TRAIT.with(|t| std::mem::replace(&mut *t.borrow_mut(), false));
    let saved_method_ref = METHOD_REF.with(|m| std::mem::take(&mut *m.borrow_mut()));
    let saved_closures = CLOSURES.with(|c| std::mem::take(&mut *c.borrow_mut()));
    let saved_anon = ANON_CLASSES.with(|c| std::mem::take(&mut *c.borrow_mut()));
    let saved_cond = COND_CLASSES.with(|c| std::mem::take(&mut *c.borrow_mut()));
    let r = prelude_classes_inner();
    // Closures the prelude declares occupy closure indices `0..n`; a program
    // with the prelude re-seeds them first. Anonymous / nested classes would
    // index per-file tables the cached copy cannot carry.
    let closures = CLOSURES.with(|c| std::mem::take(&mut *c.borrow_mut()));
    let leaked = ANON_CLASSES.with(|c| !c.borrow().is_empty())
        || COND_CLASSES.with(|c| !c.borrow().0.is_empty());
    STRICT_TYPES.with(|s| *s.borrow_mut() = saved_strict);
    SRC_FILE.with(|f| *f.borrow_mut() = saved_file);
    LINE_STARTS.with(|l| *l.borrow_mut() = saved_lines);
    NS_CTX.with(|c| *c.borrow_mut() = saved_ns);
    IN_TRAIT.with(|t| *t.borrow_mut() = saved_trait);
    METHOD_REF.with(|m| *m.borrow_mut() = saved_method_ref);
    CLOSURES.with(|c| *c.borrow_mut() = saved_closures);
    ANON_CLASSES.with(|c| *c.borrow_mut() = saved_anon);
    COND_CLASSES.with(|c| *c.borrow_mut() = saved_cond);
    assert!(
        !leaked,
        "prelude must not declare anonymous or nested classes"
    );
    let (classes, functions) = r?;
    Ok(PreludeParts {
        classes,
        functions,
        closures,
    })
}

#[allow(clippy::type_complexity)]
fn prelude_classes_inner() -> Result<(Vec<ClassProto>, Vec<Rc<FunctionProto>>), Thrown> {
    // The engine's core classes (PRELUDE) plus each extension module's own
    // PHP-source classes (GMP, Phar, …) and helper functions (iterator_*, …).
    let src = format!("{PRELUDE}\n{}", rhp_builtins::module_prelude());
    let arena = rhp_frontend::Arena::new();
    let parsed = arena.parse(b"<prelude>", src.as_bytes());
    // Method calls inside the prelude see the prelude's own by-ref signatures.
    let flat: Vec<(Rc<NsContext>, &Statement)> = parsed
        .program()
        .statements
        .iter()
        .map(|s| (Rc::new(NsContext::default()), s))
        .collect();
    METHOD_REF.with(|m| *m.borrow_mut() = collect_method_refs(&flat));
    let sigs = Rc::new(std::collections::HashMap::new());
    let mut classes = Vec::new();
    let mut functions = Vec::new();
    for stmt in parsed.program().statements.iter() {
        match stmt {
            Statement::Class(c) => classes.push(compile_class(c, &sigs)?),
            Statement::Interface(i) => classes.push(compile_interface(i, &sigs)?),
            Statement::Function(f) => functions.push(Rc::new(compile_function(f, sigs.clone())?)),
            _ => {}
        }
    }
    Ok((classes, functions))
}
