
class ReflectionException extends Exception {}

#[Attribute(Attribute::TARGET_CLASS)]
final class Attribute {
    const TARGET_CLASS = 1;
    const TARGET_FUNCTION = 2;
    const TARGET_METHOD = 4;
    const TARGET_PROPERTY = 8;
    const TARGET_CLASS_CONSTANT = 16;
    const TARGET_PARAMETER = 32;
    const TARGET_CONSTANT = 64;
    const TARGET_ALL = 127;
    const IS_REPEATABLE = 128;
    public int $flags;
    public function __construct(int $flags = Attribute::TARGET_ALL) { $this->flags = $flags; }
}
#[Attribute(Attribute::TARGET_METHOD)]
final class ReturnTypeWillChange { public function __construct() {} }
#[Attribute(Attribute::TARGET_CLASS)]
final class AllowDynamicProperties { public function __construct() {} }
#[Attribute(Attribute::TARGET_PARAMETER)]
final class SensitiveParameter { public function __construct() {} }
#[Attribute(Attribute::TARGET_METHOD|Attribute::TARGET_PROPERTY)]
final class Override { public function __construct() {} }
#[Attribute(Attribute::TARGET_METHOD|Attribute::TARGET_FUNCTION|Attribute::TARGET_CLASS_CONSTANT|Attribute::TARGET_CONSTANT|Attribute::TARGET_CLASS)]
final class Deprecated {
    public readonly ?string $message;
    public readonly ?string $since;
    public function __construct(?string $message = null, ?string $since = null) { $this->message = $message; $this->since = $since; }
}
#[Attribute(Attribute::TARGET_METHOD|Attribute::TARGET_FUNCTION)]
final class NoDiscard {
    public readonly ?string $message;
    public function __construct(?string $message = null) { $this->message = $message; }
}
#[Attribute(Attribute::TARGET_ALL)]
final class DelayedTargetValidation {}

final class SensitiveParameterValue {
    private readonly mixed $value;
    public function __construct(mixed $value) { $this->value = $value; }
    public function getValue(): mixed { return $this->value; }
    public function __debugInfo(): array { return []; }
}

class Reflection {
    public static function getModifierNames(int $modifiers): array {
        $r = [];
        if ($modifiers & 64) { $r[] = 'abstract'; }
        if ($modifiers & 32) { $r[] = 'final'; }
        if ($modifiers & 512) { $r[] = 'virtual'; }
        if ($modifiers & 1) { $r[] = 'public'; } elseif ($modifiers & 4) { $r[] = 'private'; } elseif ($modifiers & 2) { $r[] = 'protected'; }
        if ($modifiers & 2048) { $r[] = 'protected(set)'; } elseif ($modifiers & 4096) { $r[] = 'private(set)'; }
        if ($modifiers & 16) { $r[] = 'static'; }
        if ($modifiers & (128 | 65536)) { $r[] = 'readonly'; }
        return $r;
    }
    // #[\Deprecated] marks a function, method, class constant or enum case
    // ZEND_ACC_DEPRECATED.
    public static function __deprecated($descs) {
        foreach ($descs as $d) { if (strcasecmp($d[0], 'Deprecated') === 0) { return true; } }
        return false;
    }
    public static function __attrs($descs, $name, $flags, $who) {
        if ($flags & ~2) {
            throw new ValueError($who . '::getAttributes(): Argument #2 ($flags) must be a valid attribute filter flag');
        }
        $base = null;
        if ($name !== null && ($flags & 2)) {
            $base = __reflect('class', $name, true);
            if ($base === null) { throw new Error('Class "' . $name . '" not found'); }
            $base = $base['name'];
            $name = null;
        }
        $out = [];
        foreach ($descs as $d) {
            if ($name !== null) {
                if (strtolower($name) !== strtolower($d[0])) { continue; }
            } elseif ($base !== null) {
                $c = __reflect('class', $d[0], true);
                if ($c === null || !__reflect('is_a', $c['name'], $base)) { continue; }
            }
            $out[] = new ReflectionAttribute($d, $descs);
        }
        return $out;
    }
    public static function __fmt($v) {
        if ($v === null) { return 'NULL'; }
        if (is_bool($v)) { return $v ? 'true' : 'false'; }
        if (is_string($v)) { return "'" . addcslashes($v, "\\'") . "'"; }
        if (is_float($v)) {
            // smart_str_append_double with zero_frac
            $s = (string) $v;
            return is_finite($v) && strpbrk($s, '.') === false ? $s . '.0' : $s;
        }
        if (is_array($v)) {
            $list = array_is_list($v);
            $p = [];
            foreach ($v as $k => $x) { $p[] = ($list ? '' : (is_int($k) ? $k : "'" . $k . "'") . ' => ') . self::__fmt($x); }
            return '[' . implode(', ', $p) . ']';
        }
        if ($v instanceof UnitEnum) { return get_class($v) . '::' . $v->name; }
        if (is_object($v)) { return 'object(' . get_class($v) . ')'; }
        return (string) $v;
    }
    public static function __type($t) {
        if ($t === null) { return null; }
        if ($t['kind'] === 'named') { return new ReflectionNamedType($t); }
        if ($t['kind'] === 'union') { return new ReflectionUnionType($t); }
        return new ReflectionIntersectionType($t);
    }
    public static function __fn($loc) {
        return is_array($loc) ? new ReflectionMethod($loc[0], $loc[1]) : new ReflectionFunction($loc);
    }
}

interface Reflector extends Stringable {}

abstract class ReflectionType implements Stringable {
    protected $__t;
    public function __construct($t) { $this->__t = $t; }
    public function allowsNull(): bool { return $this->__t['nullable']; }
    public function __toString(): string { return ''; }
}

class ReflectionNamedType extends ReflectionType {
    public function getName(): string { return $this->__t['name']; }
    public function isBuiltin(): bool { return $this->__t['builtin']; }
    public function __toString(): string {
        $n = $this->__t['name'];
        return ($this->__t['nullable'] && $n !== 'null' && $n !== 'mixed' ? '?' : '') . $n;
    }
}

class ReflectionUnionType extends ReflectionType {
    public function getTypes(): array { $o = []; foreach ($this->__t['types'] as $t) { $o[] = Reflection::__type($t); } return $o; }
    public function __toString(): string {
        $p = [];
        foreach ($this->__t['types'] as $t) {
            $s = (string) Reflection::__type($t);
            $p[] = $t['kind'] === 'intersection' ? '(' . $s . ')' : $s;
        }
        return implode('|', $p);
    }
}

class ReflectionIntersectionType extends ReflectionType {
    public function getTypes(): array { $o = []; foreach ($this->__t['types'] as $t) { $o[] = Reflection::__type($t); } return $o; }
    public function __toString(): string { $p = []; foreach ($this->__t['types'] as $t) { $p[] = $t['name']; } return implode('&', $p); }
}

class ReflectionAttribute implements Reflector {
    const IS_INSTANCEOF = 2;
    public string $name;
    private $__d; private $__all;
    private function __construct($d, $all = []) { $this->__d = $d; $this->__all = $all; $this->name = $d[0]; }
    public function getName(): string { return $this->__d[0]; }
    public function getTarget(): int { return $this->__d[1]; }
    public function isRepeated(): bool { return $this->__d[2]; }
    public function getArguments(): array { return __reflect('attr_args', $this->__d[3]); }
    public function newInstance(): object {
        $cls = $this->__d[0];
        $c = __reflect('class', $cls, true);
        if ($c === null) { throw new Error('Attribute class "' . $cls . '" not found'); }
        $flags = null;
        foreach ($c['attrs'] as $a) {
            if (strtolower($a[0]) === 'attribute') {
                $args = __reflect('attr_args', $a[3]);
                $flags = $args[0] ?? ($args['flags'] ?? Attribute::TARGET_ALL);
                break;
            }
        }
        if ($flags === null) { throw new Error('Attempting to use non-attribute class "' . $cls . '" as attribute'); }
        $target = $this->__d[1];
        if (!($target & $flags)) {
            throw new Error('Attribute "' . $cls . '" cannot target ' . self::__targets($target) . ' (allowed targets: ' . self::__targets($flags) . ')');
        }
        if (!$c['internal'] && !($flags & 128) && $this->__d[2]) {
            throw new Error('Attribute "' . $cls . '" must not be repeated');
        }
        return new $cls(...$this->getArguments());
    }
    public static function __targets($f) {
        $n = ['class', 'function', 'method', 'property', 'class constant', 'parameter', 'constant'];
        $o = [];
        foreach ($n as $i => $s) { if ($f & (1 << $i)) { $o[] = $s; } }
        return implode(', ', $o);
    }
    public function __toString(): string { return 'Attribute [ ' . $this->__d[0] . " ]\n"; }
}

abstract class ReflectionFunctionAbstract implements Reflector {
    public string $name;
    protected $__i;
    protected $__l;
    public function inNamespace(): bool { return $this->__i['ns'] !== ''; }
    public function isClosure(): bool { return isset($this->__i['closure']); }
    public function isDeprecated(): bool { return ($this->__i['deprecated'] ?? false) || Reflection::__deprecated($this->__i['attrs']); }
    public function isInternal(): bool { return $this->__i['internal']; }
    public function isUserDefined(): bool { return !$this->__i['internal']; }
    public function isGenerator(): bool { return $this->__i['generator']; }
    public function isVariadic(): bool { return $this->__i['variadic']; }
    public function isStatic(): bool { return $this->__i['static']; }
    public function getClosureThis(): ?object { return $this->__i['this'] ?? null; }
    public function getClosureScopeClass(): ?ReflectionClass { $s = $this->__i['scope'] ?? null; return $s === null ? null : new ReflectionClass($s); }
    public function getClosureCalledClass(): ?ReflectionClass { $s = $this->__i['calledScope'] ?? null; return $s === null ? null : new ReflectionClass($s); }
    public function getClosureUsedVariables(): array { return $this->__i['used'] ?? []; }
    public function getDocComment(): string|false { return $this->__i['doc']; }
    public function getStartLine(): int|false { return $this->__i['start']; }
    public function getEndLine(): int|false { return $this->__i['end']; }
    public function getExtension(): ?ReflectionExtension { return $this->__i['ext'] === false ? null : new ReflectionExtension($this->__i['ext']); }
    public function getExtensionName(): string|false { return $this->__i['ext']; }
    public function getFileName(): string|false { return $this->__i['file']; }
    public function getName(): string { return $this->__i['name']; }
    public function getNamespaceName(): string { return $this->__i['ns']; }
    public function getNumberOfParameters(): int { return count($this->__i['params']); }
    public function getNumberOfRequiredParameters(): int { return $this->__i['nreq']; }
    public function getParameters(): array {
        $o = [];
        foreach ($this->__i['params'] as $p) { $o[] = ReflectionParameter::__make($this->__l, $this->__i, $p); }
        return $o;
    }
    public function getShortName(): string { return $this->__i['short']; }
    public function getStaticVariables(): array { return []; }
    public function returnsReference(): bool { return $this->__i['returnsRef']; }
    public function hasReturnType(): bool { return $this->__i['ret'] !== null; }
    public function getReturnType(): ?ReflectionType { return Reflection::__type($this->__i['ret']); }
    public function hasTentativeReturnType(): bool { return ($this->__i['tentative'] ?? null) !== null; }
    public function getTentativeReturnType(): ?ReflectionType { return Reflection::__type($this->__i['tentative'] ?? null); }
    public function getAttributes(?string $name = null, int $flags = 0): array { return Reflection::__attrs($this->__i['attrs'], $name, $flags, static::class); }
    public function __str() {
        $k = $this->__i['internal'] ? 'internal' : 'user';
        return ($this instanceof ReflectionMethod ? 'Method' : (isset($this->__i['closure']) ? 'Closure' : 'Function')) . ' [ <' . $k . '> ' . ($this instanceof ReflectionMethod ? implode(' ', Reflection::getModifierNames($this->__i['modifiers'])) . ' method ' : 'function ') . $this->__i['name'] . " ] {\n}\n";
    }
}

class ReflectionFunction extends ReflectionFunctionAbstract {
    const IS_DEPRECATED = 2048;
    public function __construct(Closure|string $function) {
        $i = __reflect('fn', $function);
        if ($i === null) { throw new ReflectionException('Function ' . $function . '() does not exist'); }
        $this->__i = $i;
        $this->__l = $function instanceof Closure ? $function : $i['loc'];
        $this->name = $i['name'];
    }
    public function __toString(): string { return $this->__str(); }
    public function isAnonymous(): bool { return isset($this->__i['closure']) && str_starts_with($this->__i['name'], '{closure'); }
    public function isDisabled(): bool { return false; }
    public function invoke(mixed ...$args): mixed { return call_user_func_array($this->__l, $args); }
    public function invokeArgs(array $args = []): mixed { return call_user_func_array($this->__l, $args); }
    public function getClosure(): Closure { return __reflect('closure', $this->__l, null); }
}

class ReflectionMethod extends ReflectionFunctionAbstract {
    const IS_STATIC = 16;
    const IS_PUBLIC = 1;
    const IS_PROTECTED = 2;
    const IS_PRIVATE = 4;
    const IS_ABSTRACT = 64;
    const IS_FINAL = 32;
    public string $class;
    public function __construct(object|string $objectOrMethod, ?string $method = null) {
        if (func_num_args() === 1 && !self::$__fromName) {
            __reflect('deprecated', 'Calling ReflectionMethod::__construct() with 1 argument is deprecated, use ReflectionMethod::createFromMethodName() instead');
        }
        if ($method === null) {
            if (!is_string($objectOrMethod) || !str_contains($objectOrMethod, '::')) {
                throw new ReflectionException('ReflectionMethod::__construct(): Argument #1 ($objectOrMethod) must be a valid method name');
            }
            $p = explode('::', $objectOrMethod, 2);
            $objectOrMethod = $p[0];
            $method = $p[1];
        }
        if ($objectOrMethod instanceof Closure && strcasecmp($method, '__invoke') === 0) {
            // php's closure invoke handler: an internal public method of
            // Closure with the closure's own signature.
            $i = __reflect('fn', $objectOrMethod);
            unset($i['closure'], $i['this'], $i['scope'], $i['calledScope'], $i['used']);
            $i = ['name' => '__invoke', 'short' => '__invoke', 'ns' => '', 'class' => 'Closure', 'internal' => true,
                  'ext' => 'Core', 'file' => false, 'start' => false, 'end' => false, 'doc' => false, 'attrs' => [],
                  'vis' => 1, 'modifiers' => 1, 'static' => false, 'abstract' => false, 'final' => false,
                  'prototype' => null] + $i;
            $this->__i = $i;
            $this->__l = $objectOrMethod;
            $this->name = '__invoke';
            $this->class = 'Closure';
            return;
        }
        if (is_object($objectOrMethod)) {
            $cn = __reflect('class_of', $objectOrMethod);
        } else {
            $c = __reflect('class', $objectOrMethod, true);
            if ($c === null) { throw new ReflectionException('Class "' . $objectOrMethod . '" does not exist'); }
            $cn = $c['name'];
        }
        $i = __reflect('fn', [$cn, $method]);
        if ($i === null) {
            throw new ReflectionException('Method ' . $cn . '::' . $method . '() does not exist');
        }
        $this->__i = $i;
        $this->__l = [$i['class'], $i['name']];
        $this->name = $i['name'];
        $this->class = $i['class'];
    }
    private static $__fromName = false;
    public static function createFromMethodName(string $method): static {
        self::$__fromName = true;
        try { return new static($method); } finally { self::$__fromName = false; }
    }
    public function __toString(): string { return $this->__str(); }
    public function isPublic(): bool { return $this->__i['vis'] === 1; }
    public function isPrivate(): bool { return $this->__i['vis'] === 4; }
    public function isProtected(): bool { return $this->__i['vis'] === 2; }
    public function isAbstract(): bool { return $this->__i['abstract']; }
    public function isFinal(): bool { return $this->__i['final']; }
    public function isConstructor(): bool { return strtolower($this->name) === '__construct'; }
    public function isDestructor(): bool { return strtolower($this->name) === '__destruct'; }
    public function getClosure(?object $object = null): Closure {
        if (!$this->__i['static'] && $object === null) {
            throw new TypeError('ReflectionMethod::getClosure(): Argument #1 ($object) must be provided for non-static methods');
        }
        return __reflect('closure', $this->__l, $object);
    }
    public function getModifiers(): int { return $this->__i['modifiers']; }
    // invoke() repeats invokeArgs() instead of calling it: a nested prelude
    // call would add a frame to the backtrace of what the method throws.
    public function invoke(?object $object = null, mixed ...$args): mixed {
        if ($this->__i['abstract']) { throw new ReflectionException('Trying to invoke abstract method ' . $this->class . '::' . $this->name . '()'); }
        if ($this->__i['static']) { return __reflect_invoke(null, $this->class, $this->name, $args); }
        if ($object === null) { throw new ReflectionException('Trying to invoke non static method ' . $this->class . '::' . $this->name . '() without an object'); }
        if ($this->__l instanceof Closure) {
            if (!($object instanceof Closure)) { throw new ReflectionException('Given object is not an instance of the class this method was declared in'); }
            return call_user_func_array($object, $args);
        }
        return __reflect_invoke($object, $this->class, $this->name, $args);
    }
    public function invokeArgs(?object $object = null, array $args = []): mixed {
        if ($this->__i['abstract']) { throw new ReflectionException('Trying to invoke abstract method ' . $this->class . '::' . $this->name . '()'); }
        if ($this->__i['static']) { return __reflect_invoke(null, $this->class, $this->name, $args); }
        if ($object === null) { throw new ReflectionException('Trying to invoke non static method ' . $this->class . '::' . $this->name . '() without an object'); }
        if ($this->__l instanceof Closure) {
            if (!($object instanceof Closure)) { throw new ReflectionException('Given object is not an instance of the class this method was declared in'); }
            return call_user_func_array($object, $args);
        }
        return __reflect_invoke($object, $this->class, $this->name, $args);
    }
    public function getDeclaringClass(): ReflectionClass { return new ReflectionClass($this->class); }
    public function getPrototype(): ReflectionMethod {
        $p = $this->__i['prototype'];
        if ($p === null) { throw new ReflectionException('Method ' . $this->class . '::' . $this->name . ' does not have a prototype'); }
        return new ReflectionMethod($p, $this->name);
    }
    public function hasPrototype(): bool { return $this->__i['prototype'] !== null; }
    public function setAccessible(bool $accessible): void {}
}

class ReflectionParameter implements Reflector {
    public string $name;
    private $__l; private $__f; private $__p;
    private static $__mk = null;
    public static function __make($l, $f, $p) {
        self::$__mk = [$l, $f, $p];
        try { return new ReflectionParameter(null, 0); } finally { self::$__mk = null; }
    }
    public function __construct($function, int|string $param) {
        if (self::$__mk !== null) {
            [$this->__l, $this->__f, $this->__p] = self::$__mk;
            self::$__mk = null;
        } else {
            if (is_string($function)) {
                $f = new ReflectionFunction($function);
            } elseif (is_array($function)) {
                if (!array_key_exists(0, $function) || !array_key_exists(1, $function)) {
                    throw new ReflectionException('Expected array($object, $method) or array($classname, $method)');
                }
                $c = $function[0];
                if (is_object($c)) {
                    $cn = __reflect('class_of', $c);
                } else {
                    $c = (string)$c;
                    $cr = __reflect('class', $c, true);
                    if ($cr === null) { throw new ReflectionException('Class "' . $c . '" does not exist'); }
                    $cn = $cr['name'];
                }
                $m = (string)$function[1];
                if ($c instanceof Closure && strcasecmp($m, '__invoke') === 0) {
                    $f = new ReflectionMethod($c, '__invoke');
                } else {
                    if (__reflect('fn', [$cn, $m]) === null) { throw new ReflectionException('Method ' . $cn . '::' . $m . '() does not exist'); }
                    $f = new ReflectionMethod($cn, $m);
                }
            } elseif ($function instanceof Closure) {
                $f = new ReflectionFunction($function);
            } elseif (is_object($function)) {
                $cn = __reflect('class_of', $function);
                if (__reflect('fn', [$cn, '__invoke']) === null) { throw new ReflectionException('Method ' . $cn . '::__invoke() does not exist'); }
                $f = new ReflectionMethod($cn, '__invoke');
            } else {
                $t = match (true) { $function === null => 'null', is_bool($function) => var_export($function, true), default => get_debug_type($function) };
                throw new ReflectionException('ReflectionParameter::__construct(): Argument #1 ($function) must be a string, an array(class, method), or a callable object, ' . $t . ' given');
            }
            $found = null;
            foreach ($f->getParameters() as $rp) {
                if (is_int($param) ? $rp->getPosition() === $param : $rp->name === $param) { $found = $rp; break; }
            }
            if ($found === null) { throw new ReflectionException('The parameter specified by its ' . (is_int($param) ? 'offset' : 'name') . ' could not be found'); }
            [$this->__l, $this->__f, $this->__p] = [$found->__l, $found->__f, $found->__p];
        }
        $this->name = $this->__p['name'];
    }
    public function getName(): string { return $this->__p['name']; }
    public function isPassedByReference(): bool { return $this->__p['byref']; }
    public function canBePassedByValue(): bool { return !$this->__p['byref'] || ($this->__p['preferRef'] ?? false); }
    public function getDeclaringFunction(): ReflectionFunctionAbstract { return Reflection::__fn($this->__l); }
    public function getDeclaringClass(): ?ReflectionClass {
        if (isset($this->__f['class'])) { return new ReflectionClass($this->__f['class']); }
        $s = $this->__f['scope'] ?? null;
        return $s === null ? null : new ReflectionClass($s);
    }
    public function getClass(): ?ReflectionClass {
        $t = $this->__p['type'];
        if ($t === null || $t['kind'] !== 'named' || $t['builtin']) { return null; }
        $n = $t['name'];
        if ($n === 'self' || $n === 'static') { return $this->getDeclaringClass(); }
        if ($n === 'parent') { return $this->getDeclaringClass()->getParentClass() ?: null; }
        return new ReflectionClass($n);
    }
    public function hasType(): bool { return $this->__p['type'] !== null; }
    public function getType(): ?ReflectionType { return Reflection::__type($this->__p['type']); }
    public function isArray(): bool { $t = $this->__p['type']; return $t !== null && $t['kind'] === 'named' && $t['name'] === 'array'; }
    public function isCallable(): bool { $t = $this->__p['type']; return $t !== null && $t['kind'] === 'named' && $t['name'] === 'callable'; }
    public function allowsNull(): bool { return $this->__p['type'] === null || $this->__p['type']['nullable']; }
    public function getPosition(): int { return $this->__p['pos']; }
    public function isOptional(): bool { return $this->__p['optional']; }
    public function isDefaultValueAvailable(): bool { return $this->__p['hasDefault']; }
    public function getDefaultValue(): mixed {
        if (!$this->__p['hasDefault']) { throw new ReflectionException('Internal error: Failed to retrieve the default value'); }
        return __reflect('param_default', $this->__l, $this->__p['pos']);
    }
    public function isDefaultValueConstant(): bool {
        if (!$this->__p['hasDefault']) { throw new ReflectionException('Internal error: Failed to retrieve the default value'); }
        return $this->__p['defaultConst'] !== null;
    }
    public function getDefaultValueConstantName(): ?string {
        if (!$this->__p['hasDefault']) { throw new ReflectionException('Internal error: Failed to retrieve the default value'); }
        return $this->__p['defaultConst'];
    }
    public function isVariadic(): bool { return $this->__p['variadic']; }
    public function isPromoted(): bool { return $this->__p['promoted']; }
    public function getAttributes(?string $name = null, int $flags = 0): array { return Reflection::__attrs($this->__p['attrs'], $name, $flags, 'ReflectionParameter'); }
    public function __toString(): string {
        $s = 'Parameter #' . $this->__p['pos'] . ' [ <' . ($this->__p['optional'] ? 'optional' : 'required') . '> ';
        if ($this->__p['type'] !== null) { $s .= Reflection::__type($this->__p['type']) . ' '; }
        if ($this->__p['byref']) { $s .= '&'; }
        if ($this->__p['variadic']) { $s .= '...'; }
        $s .= '$' . $this->__p['name'];
        if (array_key_exists('defaultText', $this->__p)) {
            // An internal parameter prints its arginfo default.
            if ($this->__p['optional'] && !$this->__p['variadic']) {
                $s .= ' = ' . ($this->__p['defaultText'] ?? '<default>');
            }
        } elseif ($this->__p['optional'] && !$this->__p['variadic'] && $this->__p['hasDefault']) {
            $s .= ' = ' . ($this->__p['defaultConst'] ?? Reflection::__fmt($this->getDefaultValue()));
        }
        return $s . ' ]';
    }
}

class ReflectionClass implements Reflector {
    const IS_IMPLICIT_ABSTRACT = 16;
    const IS_EXPLICIT_ABSTRACT = 64;
    const IS_FINAL = 32;
    const IS_READONLY = 65536;
    const SKIP_INITIALIZATION_ON_SERIALIZE = 8;
    const SKIP_DESTRUCTOR = 16;
    public string $name;
    protected $__c;
    protected $__o;
    public function __construct(object|string $objectOrClass) {
        if (is_object($objectOrClass)) {
            // Only a ReflectionObject keeps the object (php's intern->obj):
            // dynamic properties and the closure's __invoke come from it.
            if ($this instanceof ReflectionObject) { $this->__o = $objectOrClass; }
            $objectOrClass = __reflect('class_of', $objectOrClass);
        }
        $c = __reflect('class', $objectOrClass, true);
        if ($c === null) { throw new ReflectionException('Class "' . $objectOrClass . '" does not exist'); }
        $this->__c = $c;
        $this->name = $c['name'];
    }
    public function __toString(): string { return 'Class [ <' . ($this->__c['internal'] ? 'internal' : 'user') . '> class ' . $this->name . " ] {\n}\n"; }
    public function getName(): string { return $this->__c['name']; }
    public function isInternal(): bool { return $this->__c['internal']; }
    public function isUserDefined(): bool { return !$this->__c['internal']; }
    public function isAnonymous(): bool { return $this->__c['anon']; }
    public function isInstantiable(): bool { return $this->__c['instantiable']; }
    public function isCloneable(): bool { return $this->__c['cloneable']; }
    public function getFileName(): string|false { return $this->__c['file']; }
    public function getStartLine(): int|false { return $this->__c['start']; }
    public function getEndLine(): int|false { return $this->__c['end']; }
    public function getDocComment(): string|false { return $this->__c['doc']; }
    public function getConstructor(): ?ReflectionMethod { return $this->hasMethod('__construct') ? new ReflectionMethod($this->name, '__construct') : null; }
    public function hasMethod(string $name): bool {
        if ($this->name === 'Closure' && strcasecmp($name, '__invoke') === 0) { return true; }
        return __reflect('fn', [$this->name, $name]) !== null;
    }
    public function getMethod(string $name): ReflectionMethod {
        if ($this->__o instanceof Closure && strcasecmp($name, '__invoke') === 0) { return new ReflectionMethod($this->__o, '__invoke'); }
        if (__reflect('fn', [$this->name, $name]) !== null) { return new ReflectionMethod($this->__o ?? $this->name, $name); }
        throw new ReflectionException('Method ' . $this->name . '::' . $name . '() does not exist');
    }
    public function getMethods(?int $filter = null): array {
        $o = [];
        foreach (__reflect('methods', $this->name) as $m) {
            $rm = new ReflectionMethod($this->name, $m[0]);
            if ($filter === null || ($rm->getModifiers() & $filter)) { $o[] = $rm; }
        }
        return $o;
    }
    public function hasProperty(string $name): bool {
        foreach (__reflect('props', $this->name) as $p) { if ($p[0] === $name) { return true; } }
        if ($this->__o !== null) { return in_array($name, __reflect('obj_props', $this->__o), true); }
        return false;
    }
    public function getProperty(string $name): ReflectionProperty {
        if ($this->hasProperty($name)) { return new ReflectionProperty($this->__o ?? $this->name, $name); }
        throw new ReflectionException('Property ' . $this->name . '::$' . $name . ' does not exist');
    }
    public function getProperties(?int $filter = null): array {
        $o = [];
        foreach (__reflect('props', $this->name) as $p) {
            $rp = new ReflectionProperty($this->name, $p[0]);
            if ($filter === null || ($rp->getModifiers() & $filter)) { $o[] = $rp; }
        }
        if ($this->__o !== null && ($filter === null || ($filter & 1))) {
            foreach (__reflect('obj_props', $this->__o) as $n) { $o[] = new ReflectionProperty($this->__o, $n); }
        }
        return $o;
    }
    public function hasConstant(string $name): bool {
        foreach (__reflect('consts', $this->name) as $c) { if ($c[0] === $name) { return true; } }
        return false;
    }
    public function getConstants(?int $filter = null): array {
        $o = [];
        foreach (__reflect('consts', $this->name) as $c) {
            $i = __reflect('const', $this->name, $c[0]);
            if ($filter === null || ($i['modifiers'] & $filter)) { $o[$c[0]] = $i['value']; }
        }
        return $o;
    }
    public function getReflectionConstants(?int $filter = null): array {
        $o = [];
        foreach (__reflect('consts', $this->name) as $c) {
            $rc = new ReflectionClassConstant($this->name, $c[0]);
            if ($filter === null || ($rc->getModifiers() & $filter)) { $o[] = $rc; }
        }
        return $o;
    }
    public function getConstant(string $name): mixed {
        $i = __reflect('const', $this->name, $name);
        if ($i === null) {
            __reflect('deprecated', 'ReflectionClass::getConstant() for a non-existent constant is deprecated, use ReflectionClass::hasConstant() to check if the constant exists');
            return false;
        }
        return $i['value'];
    }
    public function getReflectionConstant(string $name): ReflectionClassConstant|false {
        return $this->hasConstant($name) ? new ReflectionClassConstant($this->name, $name) : false;
    }
    public function getInterfaces(): array { $o = []; foreach ($this->__c['interfaces'] as $i) { $o[$i] = new ReflectionClass($i); } return $o; }
    public function getInterfaceNames(): array { return $this->__c['interfaces']; }
    public function isInterface(): bool { return $this->__c['kind'] === 'interface'; }
    public function getTraits(): array { $o = []; foreach ($this->__c['traits'] as $t) { $o[$t] = new ReflectionClass($t); } return $o; }
    public function getTraitNames(): array { return $this->__c['traits']; }
    public function getTraitAliases(): array { return []; }
    public function isTrait(): bool { return $this->__c['kind'] === 'trait'; }
    public function isEnum(): bool { return $this->__c['kind'] === 'enum'; }
    public function isAbstract(): bool { return $this->__c['abstract']; }
    public function isFinal(): bool { return $this->__c['final']; }
    public function isReadOnly(): bool { return $this->__c['readonly']; }
    public function getModifiers(): int { return $this->__c['modifiers']; }
    public function isInstance(object $object): bool { return $object instanceof $this->name; }
    public function newInstance(mixed ...$args): object {
        $ctor = __reflect('fn', [$this->name, '__construct']);
        if ($ctor === null) {
            if (count($args) > 0) { throw new ReflectionException('Class ' . $this->name . ' does not have a constructor, so you cannot pass any constructor arguments'); }
            return __reflect_new_instance_noctor($this->name);
        }
        if ($ctor['vis'] !== 1) { throw new ReflectionException('Access to non-public constructor of class ' . $this->name); }
        $cls = $this->name;
        return new $cls(...$args);
    }
    public function newInstanceWithoutConstructor(): object {
        if ($this->__c['internal'] && $this->isFinal() && $this->__c['create_object']) {
            throw new ReflectionException('Class ' . $this->name . ' is an internal class marked as final that cannot be instantiated without invoking its constructor');
        }
        return __reflect_new_instance_noctor($this->name);
    }
    public function newInstanceArgs(array $args = []): ?object {
        $ctor = __reflect('fn', [$this->name, '__construct']);
        if ($ctor === null) {
            if (count($args) > 0) { throw new ReflectionException('Class ' . $this->name . ' does not have a constructor, so you cannot pass any constructor arguments'); }
            return __reflect_new_instance_noctor($this->name);
        }
        if ($ctor['vis'] !== 1) { throw new ReflectionException('Access to non-public constructor of class ' . $this->name); }
        $cls = $this->name;
        return new $cls(...$args);
    }
    public function newLazyGhost(callable $initializer, int $options = 0): object { throw new Error('Lazy objects are not supported'); }
    public function newLazyProxy(callable $factory, int $options = 0): object { throw new Error('Lazy objects are not supported'); }
    public function resetAsLazyGhost(object $object, callable $initializer, int $options = 0): void { throw new Error('Lazy objects are not supported'); }
    public function resetAsLazyProxy(object $object, callable $factory, int $options = 0): void { throw new Error('Lazy objects are not supported'); }
    public function isUninitializedLazyObject(object $object): bool { return false; }
    public function initializeLazyObject(object $object): object { return $object; }
    public function markLazyObjectAsInitialized(object $object): object { return $object; }
    public function getLazyInitializer(object $object): ?callable { return null; }
    public function getParentClass(): ReflectionClass|false { return $this->__c['parent'] === false ? false : new ReflectionClass($this->__c['parent']); }
    public function isSubclassOf(ReflectionClass|string $class): bool {
        if ($class instanceof ReflectionClass) { $class = $class->name; }
        $c = __reflect('class', $class, true);
        if ($c === null) { throw new ReflectionException('Class "' . $class . '" does not exist'); }
        return strcasecmp($c['name'], $this->name) !== 0 && __reflect('is_a', $this->name, $c['name']);
    }
    public function getStaticProperties(): array { return __reflect('static_props', $this->name); }
    public function getStaticPropertyValue(string $name, mixed $default = null): mixed {
        $all = __reflect('static_props', $this->name);
        if (array_key_exists($name, $all)) { return $all[$name]; }
        if (func_num_args() > 1) { return $default; }
        throw new ReflectionException('Property ' . $this->name . '::$' . $name . ' does not exist');
    }
    public function setStaticPropertyValue(string $name, mixed $value): void {
        if (!__reflect('set_static', $this->name, $name, $value)) {
            throw new ReflectionException('Class ' . $this->name . ' does not have a property named ' . $name);
        }
    }
    public function getDefaultProperties(): array { return __reflect('default_props', $this->name, false); }
    public function isIterable(): bool { return $this->__c['iterable']; }
    public function isIterateable(): bool { return $this->__c['iterable']; }
    public function implementsInterface(ReflectionClass|string $interface): bool {
        if ($interface instanceof ReflectionClass) { $interface = $interface->name; }
        $c = __reflect('class', $interface, true);
        if ($c === null) { throw new ReflectionException('Interface "' . $interface . '" does not exist'); }
        if ($c['kind'] !== 'interface') { throw new ReflectionException($c['name'] . ' is not an interface'); }
        return __reflect('is_a', $this->name, $c['name']);
    }
    public function getExtension(): ?ReflectionExtension { return $this->__c['ext'] === false ? null : new ReflectionExtension($this->__c['ext']); }
    public function getExtensionName(): string|false { return $this->__c['ext']; }
    public function inNamespace(): bool { return $this->__c['ns'] !== ''; }
    public function getNamespaceName(): string { return $this->__c['ns']; }
    public function getShortName(): string { return $this->__c['short']; }
    public function getAttributes(?string $name = null, int $flags = 0): array { return Reflection::__attrs($this->__c['attrs'], $name, $flags, 'ReflectionClass'); }
}

class ReflectionObject extends ReflectionClass {
    public function __construct(object $object) { parent::__construct($object); }
}

class ReflectionEnum extends ReflectionClass {
    public function __construct(object|string $objectOrClass) {
        parent::__construct($objectOrClass);
        if ($this->__c['kind'] !== 'enum') { throw new ReflectionException('Class "' . $this->name . '" is not an enum', -1); }
    }
    public function hasCase(string $name): bool { return in_array($name, $this->__c['cases'], true); }
    public function getCase(string $name): ReflectionEnumUnitCase {
        if (!$this->hasCase($name)) {
            if ($this->hasConstant($name)) { throw new ReflectionException($this->name . '::' . $name . ' is not a case'); }
            throw new ReflectionException('Case ' . $this->name . '::' . $name . ' does not exist');
        }
        return $this->__c['backing'] === null ? new ReflectionEnumUnitCase($this->name, $name) : new ReflectionEnumBackedCase($this->name, $name);
    }
    public function getCases(): array { $o = []; foreach ($this->__c['cases'] as $c) { $o[] = $this->getCase($c); } return $o; }
    public function isBacked(): bool { return $this->__c['backing'] !== null; }
    public function getBackingType(): ?ReflectionNamedType {
        $b = $this->__c['backing'];
        return $b === null ? null : new ReflectionNamedType(['kind' => 'named', 'name' => $b, 'nullable' => false, 'builtin' => true]);
    }
}

class ReflectionProperty implements Reflector {
    const IS_STATIC = 16;
    const IS_READONLY = 128;
    const IS_PUBLIC = 1;
    const IS_PROTECTED = 2;
    const IS_PRIVATE = 4;
    const IS_ABSTRACT = 64;
    const IS_PROTECTED_SET = 2048;
    const IS_PRIVATE_SET = 4096;
    const IS_VIRTUAL = 512;
    const IS_FINAL = 32;
    public string $name;
    public string $class;
    private $__i;
    public function __construct(object|string $class, string $property) {
        if (is_object($class)) {
            $obj = $class;
            $cn = __reflect('class_of', $class);
        } else {
            $obj = null;
            $c = __reflect('class', $class, true);
            if ($c === null) { throw new ReflectionException('Class "' . $class . '" does not exist'); }
            $cn = $c['name'];
        }
        $i = __reflect('prop', $cn, $property);
        if ($i === null) {
            if ($obj !== null && in_array($property, __reflect('obj_props', $obj), true)) {
                $i = ['name' => $property, 'class' => $cn, 'static' => false, 'vis' => 1, 'modifiers' => 1, 'readonly' => false,
                      'promoted' => false, 'type' => null, 'hasDefault' => false, 'default' => null, 'doc' => false, 'attrs' => [], 'dynamic' => true];
            } else {
                throw new ReflectionException('Property ' . $cn . '::$' . $property . ' does not exist');
            }
        }
        $this->__i = $i;
        $this->name = $i['name'];
        $this->class = $i['class'];
    }
    public function __toString(): string { return 'Property [ ' . implode(' ', Reflection::getModifierNames($this->__i['modifiers'])) . ' $' . $this->name . " ]\n"; }
    public function getName(): string { return $this->name; }
    public function getMangledName(): string {
        $v = $this->__i['vis'];
        if ($v === 4) { return "\0" . $this->class . "\0" . $this->name; }
        if ($v === 2) { return "\0*\0" . $this->name; }
        return $this->name;
    }
    public function getValue(?object $object = null): mixed {
        if ($this->__i['static']) { return __reflect('static_props', $this->class)[$this->name] ?? null; }
        if ($object === null) { throw new TypeError('ReflectionProperty::getValue(): Argument #1 ($object) must be provided for instance properties'); }
        if (!($object instanceof $this->class)) { throw new ReflectionException('Given object is not an instance of the class this property was declared in'); }
        if ($this->__i['type'] !== null && !isset($this->__i['dynamic']) && !__reflect('is_init', $object, $this->class, $this->name)) {
            throw new Error('Typed property ' . $this->class . '::$' . $this->name . ' must not be accessed before initialization');
        }
        return __reflect_get_prop($object, $this->name, $this->class);
    }
    public function setValue(mixed $objectOrValue, mixed $value = null): void {
        if ($this->__i['static']) {
            if (func_num_args() === 1) {
                __reflect('deprecated', 'Calling ReflectionProperty::setValue() with a single argument is deprecated');
                $value = $objectOrValue;
            } elseif ($objectOrValue !== null && !is_object($objectOrValue)) {
                __reflect('deprecated', 'Calling ReflectionProperty::setValue() with a 1st argument which is not null or an object is deprecated');
            }
            __reflect('set_static', $this->class, $this->name, $value);
            return;
        }
        if (func_num_args() < 2) { throw new ArgumentCountError('ReflectionProperty::setValue() expects exactly 2 arguments, ' . func_num_args() . ' given'); }
        if (!is_object($objectOrValue)) { throw new TypeError('ReflectionProperty::setValue(): Argument #1 ($objectOrValue) must be of type object, ' . get_debug_type($objectOrValue) . ' given'); }
        __reflect_set_prop($objectOrValue, $this->name, $value, $this->class);
    }
    public function getRawValue(object $object): mixed { return $this->getValue($object); }
    public function setRawValue(object $object, mixed $value): void { $this->setValue($object, $value); }
    public function setRawValueWithoutLazyInitialization(object $object, mixed $value): void { $this->setValue($object, $value); }
    public function skipLazyInitialization(object $object): void {}
    public function isLazy(object $object): bool { return false; }
    public function isInitialized(?object $object = null): bool {
        if ($this->__i['static']) { return true; }
        if ($object === null) { throw new TypeError('ReflectionProperty::isInitialized(): Argument #1 ($object) must be provided for instance properties'); }
        if (!($object instanceof $this->class)) { throw new ReflectionException('Given object is not an instance of the class this property was declared in'); }
        return __reflect('is_init', $object, $this->class, $this->name);
    }
    public function isPublic(): bool { return $this->__i['vis'] === 1; }
    public function isPrivate(): bool { return $this->__i['vis'] === 4; }
    public function isProtected(): bool { return $this->__i['vis'] === 2; }
    public function isPrivateSet(): bool { return ($this->__i['modifiers'] & 4096) !== 0; }
    public function isProtectedSet(): bool { return ($this->__i['modifiers'] & 2048) !== 0; }
    public function isStatic(): bool { return $this->__i['static']; }
    public function isReadOnly(): bool { return $this->__i['readonly']; }
    public function isDefault(): bool { return !isset($this->__i['dynamic']); }
    public function isDynamic(): bool { return isset($this->__i['dynamic']); }
    public function isAbstract(): bool { return false; }
    public function isVirtual(): bool { return false; }
    public function isPromoted(): bool { return $this->__i['promoted']; }
    public function getModifiers(): int { return $this->__i['modifiers']; }
    public function getDeclaringClass(): ReflectionClass { return new ReflectionClass($this->class); }
    public function getDocComment(): string|false { return $this->__i['doc']; }
    public function setAccessible(bool $accessible): void {}
    public function getType(): ?ReflectionType { return Reflection::__type($this->__i['type']); }
    public function getSettableType(): ?ReflectionType { return Reflection::__type($this->__i['type']); }
    public function hasType(): bool { return $this->__i['type'] !== null; }
    public function hasDefaultValue(): bool { return $this->__i['hasDefault']; }
    public function getDefaultValue(): mixed {
        if (!$this->__i['hasDefault']) {
            __reflect('deprecated', 'ReflectionProperty::getDefaultValue() for a property without a default value is deprecated, use ReflectionProperty::hasDefaultValue() to check if the default value exists');
            return null;
        }
        return $this->__i['default'];
    }
    public function getAttributes(?string $name = null, int $flags = 0): array { return Reflection::__attrs($this->__i['attrs'], $name, $flags, 'ReflectionProperty'); }
    public function hasHooks(): bool { return false; }
    public function getHooks(): array { return []; }
    public function hasHook($type): bool { return false; }
    public function getHook($type): ?ReflectionMethod { return null; }
    public function isFinal(): bool { return ($this->__i['modifiers'] & 32) !== 0; }
}

class ReflectionClassConstant implements Reflector {
    const IS_PUBLIC = 1;
    const IS_PROTECTED = 2;
    const IS_PRIVATE = 4;
    const IS_FINAL = 32;
    public string $name;
    public string $class;
    protected $__i;
    public function __construct(object|string $class, string $constant) {
        $cn = is_object($class) ? __reflect('class_of', $class) : $class;
        $c = __reflect('class', $cn, true);
        if ($c === null) { throw new ReflectionException('Class "' . $cn . '" does not exist'); }
        $i = __reflect('const', $c['name'], $constant);
        if ($i === null) { throw new ReflectionException('Constant ' . $c['name'] . '::' . $constant . ' does not exist'); }
        $this->__i = $i;
        $this->name = $i['name'];
        $this->class = $i['class'];
    }
    public function __toString(): string { return 'Constant [ ' . implode(' ', Reflection::getModifierNames($this->__i['modifiers'])) . ' ' . get_debug_type($this->__i['value']) . ' ' . $this->name . ' ] { ' . (is_scalar($this->__i['value']) ? $this->__i['value'] : '') . " }\n"; }
    public function getName(): string { return $this->name; }
    public function getValue(): mixed { return $this->__i['value']; }
    public function isPublic(): bool { return $this->__i['vis'] === 1; }
    public function isPrivate(): bool { return $this->__i['vis'] === 4; }
    public function isProtected(): bool { return $this->__i['vis'] === 2; }
    public function isFinal(): bool { return $this->__i['final']; }
    public function getModifiers(): int { return $this->__i['modifiers']; }
    public function getDeclaringClass(): ReflectionClass { return new ReflectionClass($this->class); }
    public function getDocComment(): string|false { return $this->__i['doc']; }
    public function getAttributes(?string $name = null, int $flags = 0): array { return Reflection::__attrs($this->__i['attrs'], $name, $flags, 'ReflectionClassConstant'); }
    public function isEnumCase(): bool { return $this->__i['case']; }
    public function isDeprecated(): bool { return Reflection::__deprecated($this->__i['attrs']); }
    public function hasType(): bool { return $this->__i['type'] !== null; }
    public function getType(): ?ReflectionType { return Reflection::__type($this->__i['type']); }
}

class ReflectionEnumUnitCase extends ReflectionClassConstant {
    public function __construct(object|string $class, string $constant) {
        parent::__construct($class, $constant);
        if (!$this->__i['case']) { throw new ReflectionException('Constant ' . $this->class . '::' . $constant . ' is not a case'); }
    }
    public function getEnum(): ReflectionEnum { return new ReflectionEnum($this->class); }
    public function getValue(): UnitEnum { return $this->__i['value']; }
}

class ReflectionEnumBackedCase extends ReflectionEnumUnitCase {
    public function getBackingValue(): int|string { return $this->__i['value']->value; }
}

class ReflectionConstant implements Reflector {
    public string $name;
    public function __construct(string $name) {
        $n = ltrim($name, '\\');
        if (!defined($n)) { throw new ReflectionException('Constant "' . $name . '" does not exist'); }
        $this->name = $n;
    }
    public function getName(): string { return $this->name; }
    public function getNamespaceName(): string { $p = strrpos($this->name, '\\'); return $p === false ? '' : substr($this->name, 0, $p); }
    public function getShortName(): string { $p = strrpos($this->name, '\\'); return $p === false ? $this->name : substr($this->name, $p + 1); }
    public function getValue(): mixed { return constant($this->name); }
    public function isDeprecated(): bool { return false; }
    public function getFileName(): string|false { return false; }
    public function getExtension(): ?ReflectionExtension { return null; }
    public function getExtensionName(): string|false { return false; }
    public function getAttributes(?string $name = null, int $flags = 0): array { return []; }
    public function __toString(): string { return 'Constant [ ' . get_debug_type($this->getValue()) . ' ' . $this->name . ' ] { ' . (is_scalar($this->getValue()) ? $this->getValue() : '') . " }\n"; }
}

final class ReflectionReference {
    private $__id;
    private function __construct($id) { $this->__id = $id; }
    public static function fromArrayElement(array $array, int|string $key): ?ReflectionReference {
        if (!array_key_exists($key, $array)) { throw new ReflectionException('Array key not found'); }
        $id = __reflect('ref_id', $array, $key);
        return $id === null ? null : new ReflectionReference($id);
    }
    public function getId(): string { return $this->__id; }
}

final class ReflectionGenerator {
    private $__g;
    public function __construct(Generator $generator) { $this->__g = $generator; }
    public function getExecutingLine(): int { return 0; }
    public function getExecutingFile(): string { return ''; }
    public function getTrace(int $options = DEBUG_BACKTRACE_PROVIDE_OBJECT): array { return []; }
    public function getFunction(): ReflectionFunctionAbstract { throw new ReflectionException('Cannot fetch information from a terminated Generator'); }
    public function getThis(): ?object { return null; }
    public function getExecutingGenerator(): Generator { return $this->__g; }
    public function isClosed(): bool { return !$this->__g->valid(); }
}

final class ReflectionFiber {
    private $__f;
    public function __construct(Fiber $fiber) { $this->__f = $fiber; }
    public function getFiber(): Fiber { return $this->__f; }
    public function getExecutingFile(): ?string { return null; }
    public function getExecutingLine(): ?int { return null; }
    public function getCallable(): callable { throw new Error('Cannot fetch the callable from a fiber that has terminated'); }
    public function getTrace(int $options = DEBUG_BACKTRACE_PROVIDE_OBJECT): array { return []; }
}

class ReflectionExtension implements Reflector {
    public string $name;
    public function __construct(string $name) {
        if (!extension_loaded($name)) { throw new ReflectionException('Extension "' . $name . '" does not exist'); }
        foreach (get_loaded_extensions() as $e) { if (strcasecmp($e, $name) === 0) { $name = $e; break; } }
        $this->name = $name;
    }
    public function getName(): string { return $this->name; }
    public function getVersion(): ?string { $v = phpversion($this->name); return $v === false ? null : $v; }
    public function info(): void {}
    public function getClasses(): array { return []; }
    public function getClassNames(): array { return []; }
    public function getConstants(): array { return []; }
    public function getFunctions(): array { return []; }
    public function getINIEntries(): array { return []; }
    public function getDependencies(): array { return []; }
    public function isPersistent(): bool { return true; }
    public function isTemporary(): bool { return false; }
    public function __toString(): string { return 'Extension [ <persistent> extension #0 ' . $this->name . " ] {\n}\n"; }
}
