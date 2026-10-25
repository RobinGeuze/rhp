//! A pragmatic subset of the `Reflection` extension — enough for Symfony
//! Console / Composer command introspection. The `Reflection*` classes live in
//! `PRELUDE_PHP` and lean on PHP itself (`call_user_func_array`, `get_class`)
//! wherever possible; only the genuinely-native bits (class metadata, visibility-
//! bypassing property access, instantiation, parameter descriptors) delegate to
//! `__reflect_*` builtins backed by `Host`.

use std::collections::HashMap;

use rhp_values::{conv, Value};

use crate::{Builtin, Host, Thrown};

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

fn str_arg(args: &[Value], i: usize) -> Vec<u8> {
    conv::to_string_bytes(&deref(&args[i]))
}

fn fn_meta(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(host.reflect_meta(&str_arg(args, 0)).unwrap_or(Value::Null))
}

fn fn_is_a(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_arg(args, 0);
    let other = str_arg(args, 1);
    let other = other.strip_prefix(b"\\").unwrap_or(&other);
    Ok(Value::Bool(host.reflect_is_a(&name, other)))
}

fn args_vec(v: &Value) -> Vec<Value> {
    match deref(v) {
        Value::Array(a) => a.iter().map(|(_, v)| v.clone()).collect(),
        _ => Vec::new(),
    }
}

fn fn_new_instance(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_arg(args, 0);
    let a = args.get(1).map(args_vec).unwrap_or_default();
    host.reflect_new_instance(&name, a, true)
}

fn fn_new_instance_noctor(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = str_arg(args, 0);
    host.reflect_new_instance(&name, Vec::new(), false)
}

fn fn_get_prop(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let obj = deref(&args[0]);
    let name = str_arg(args, 1);
    Ok(host.object_prop(&obj, &name).unwrap_or(Value::Null))
}

fn fn_set_prop(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let obj = deref(&args[0]);
    let name = str_arg(args, 1);
    let val = deref(&args[2]);
    host.reflect_set_prop(&obj, &name, val);
    Ok(Value::Null)
}

fn fn_method_params(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let class = str_arg(args, 0);
    let method = str_arg(args, 1);
    Ok(host.reflect_method_params(&class, &method).unwrap_or_else(|| Value::Array(rhp_values::PhpArray::new())))
}

fn fn_func_params(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let c = deref(&args[0]);
    Ok(host.reflect_callable_params(&c).unwrap_or_else(|| Value::Array(rhp_values::PhpArray::new())))
}

fn fn_class_constants(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    Ok(host
        .reflect_class_constants(&str_arg(args, 0))
        .unwrap_or_else(|| Value::Array(rhp_values::PhpArray::new())))
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(name, Builtin { name, func, min_args: min, max_args: max });
    }
    put(m, "__reflect_meta", fn_meta, 1, Some(1));
    put(m, "__reflect_is_a", fn_is_a, 2, Some(2));
    put(m, "__reflect_new_instance", fn_new_instance, 1, Some(2));
    put(m, "__reflect_new_instance_noctor", fn_new_instance_noctor, 1, Some(1));
    put(m, "__reflect_get_prop", fn_get_prop, 2, Some(2));
    put(m, "__reflect_set_prop", fn_set_prop, 3, Some(3));
    put(m, "__reflect_method_params", fn_method_params, 2, Some(2));
    put(m, "__reflect_func_params", fn_func_params, 1, Some(1));
    put(m, "__reflect_class_constants", fn_class_constants, 1, Some(1));
}

/// The `Reflection*` classes (read-focused subset).
pub const PRELUDE_PHP: &str = r#"
class ReflectionException extends Exception {}

class ReflectionNamedType {
    private $tname; private $tnullable; private $tbuiltin;
    public function __construct($n, $nu, $b) { $this->tname = $n; $this->tnullable = $nu; $this->tbuiltin = $b; }
    public function getName() { return $this->tname; }
    public function allowsNull() { return $this->tnullable; }
    public function isBuiltin() { return $this->tbuiltin; }
    public function __toString() { return ($this->tnullable && $this->tbuiltin && $this->tname !== 'null' && $this->tname !== 'mixed' ? '?' : '') . $this->tname; }
}

class ReflectionParameter {
    public $name;
    private $meta;
    public function __construct($meta) { $this->meta = $meta; $this->name = $meta['name']; }
    public function getName() { return $this->name; }
    public function isOptional() { return $this->meta['optional']; }
    public function isVariadic() { return $this->meta['variadic']; }
    public function isPassedByReference() { return $this->meta['byref']; }
    public function isDefaultValueAvailable() { return $this->meta['hasDefault']; }
    public function getDefaultValue() { return $this->meta['default'] ?? null; }
    public function hasType() { return $this->meta['hasType']; }
    public function getType() { return $this->meta['hasType'] ? new ReflectionNamedType($this->meta['type'], $this->meta['typeNullable'], $this->meta['typeBuiltin']) : null; }
    public function allowsNull() { return !$this->meta['hasType'] || $this->meta['typeNullable']; }
}

class ReflectionMethod {
    public $name; public $class;
    public function __construct($class, $name = null) {
        if ($name === null) { $p = explode('::', $class); $class = $p[0]; $name = $p[1]; }
        $this->class = is_object($class) ? get_class($class) : $class;
        $this->name = $name;
    }
    public function getName() { return $this->name; }
    public function getShortName() { return $this->name; }
    public function getDeclaringClass() { return new ReflectionClass($this->class); }
    public function invoke($object, ...$args) { return call_user_func_array([$object ?? $this->class, $this->name], $args); }
    public function invokeArgs($object, $args) { return call_user_func_array([$object ?? $this->class, $this->name], $args); }
    public function setAccessible($a) {}
    public function getParameters() { $out = []; foreach (__reflect_method_params($this->class, $this->name) as $m) { $out[] = new ReflectionParameter($m); } return $out; }
    public function getNumberOfParameters() { return count(__reflect_method_params($this->class, $this->name)); }
    public function getNumberOfRequiredParameters() { $n = 0; foreach (__reflect_method_params($this->class, $this->name) as $m) { if (!$m['optional']) $n++; } return $n; }
}

class ReflectionProperty {
    public $name; public $class;
    public function __construct($class, $name) { $this->class = is_object($class) ? get_class($class) : $class; $this->name = $name; }
    public function getName() { return $this->name; }
    public function setAccessible($a) {}
    public function getValue($object = null) { return __reflect_get_prop($object, $this->name); }
    public function setValue($object, $value = null) { __reflect_set_prop($object, $this->name, $value); }
    public function isInitialized($object = null) { return true; }
}

class ReflectionFunction {
    public $name; private $callable;
    public function __construct($f) { $this->callable = $f; $this->name = is_string($f) ? $f : '{closure}'; }
    public function getName() { return $this->name; }
    public function invoke(...$args) { return call_user_func_array($this->callable, $args); }
    public function invokeArgs($args) { return call_user_func_array($this->callable, $args); }
    public function getParameters() { $out = []; foreach (__reflect_func_params($this->callable) as $m) { $out[] = new ReflectionParameter($m); } return $out; }
    public function getNumberOfParameters() { return count(__reflect_func_params($this->callable)); }
    public function getNumberOfRequiredParameters() { $n = 0; foreach (__reflect_func_params($this->callable) as $m) { if (!$m['optional']) $n++; } return $n; }
}

class ReflectionClass {
    public $name;
    private $meta;
    public function __construct($arg) {
        $this->name = is_object($arg) ? get_class($arg) : $arg;
        $m = __reflect_meta($this->name);
        if ($m === null) { throw new ReflectionException('Class "' . $this->name . '" does not exist'); }
        $this->meta = $m;
        $this->name = $m['name'];
    }
    public function getName() { return $this->name; }
    public function getShortName() { return $this->meta['shortName']; }
    public function getNamespaceName() { return $this->meta['namespaceName']; }
    public function inNamespace() { return $this->meta['namespaceName'] !== ''; }
    public function isAbstract() { return $this->meta['isAbstract']; }
    public function isInterface() { return $this->meta['isInterface']; }
    public function isInstantiable() { return $this->meta['isInstantiable']; }
    public function isFinal() { return $this->meta['isFinal']; }
    public function isEnum() { return $this->meta['isEnum']; }
    public function isInternal() { return false; }
    public function isUserDefined() { return true; }
    public function getParentClass() { return $this->meta['parent'] === false ? false : new ReflectionClass($this->meta['parent']); }
    public function getInterfaceNames() { return $this->meta['interfaces']; }
    public function implementsInterface($i) { return __reflect_is_a($this->name, $i); }
    public function isSubclassOf($c) { return __reflect_is_a($this->name, $c) && strcasecmp(ltrim($this->name, '\\'), ltrim($c, '\\')) !== 0; }
    public function isInstance($obj) { return __reflect_is_a(get_class($obj), $this->name); }
    public function hasMethod($m) { return in_array(strtolower($m), array_map('strtolower', $this->meta['methods'])); }
    public function getMethod($m) { return new ReflectionMethod($this->name, $m); }
    public function getMethods($filter = null) { $out = []; foreach ($this->meta['methods'] as $m) { $out[] = new ReflectionMethod($this->name, $m); } return $out; }
    public function getConstructor() { return $this->hasMethod('__construct') ? new ReflectionMethod($this->name, '__construct') : null; }
    public function getConstants() { return $this->meta['constants']; }
    public function getConstant($n) { return $this->meta['constants'][$n] ?? false; }
    public function hasConstant($n) { return array_key_exists($n, $this->meta['constants']); }
    public function getReflectionConstants($filter = null) { $out = []; foreach (__reflect_class_constants($this->name) as $c) { $out[] = new ReflectionClassConstant($this->name, $c['name']); } return $out; }
    public function getReflectionConstant($n) { return $this->hasConstant($n) ? new ReflectionClassConstant($this->name, $n) : false; }
    public function getProperties($filter = null) { $out = []; foreach ($this->meta['properties'] as $p) { $out[] = new ReflectionProperty($this->name, $p); } return $out; }
    public function getProperty($p) { return new ReflectionProperty($this->name, $p); }
    public function hasProperty($p) { return in_array($p, $this->meta['properties']); }
    public function newInstance(...$args) { return __reflect_new_instance($this->name, $args); }
    public function newInstanceArgs($args = []) { return __reflect_new_instance($this->name, $args); }
    public function newInstanceWithoutConstructor() { return __reflect_new_instance_noctor($this->name); }
    public function getAttributes($name = null, $flags = 0) { return []; }
    public function getInterfaces() { $out = []; foreach ($this->meta['interfaces'] as $i) { $out[$i] = new ReflectionClass($i); } return $out; }
}

class ReflectionObject extends ReflectionClass {}
class ReflectionEnum extends ReflectionClass {}

class ReflectionClassConstant {
    public $name; public $class; private $__m;
    public function __construct($class, $name) {
        $this->class = is_object($class) ? get_class($class) : $class;
        $this->name = $name;
        $this->__m = null;
    }
    private function meta() {
        if ($this->__m === null) {
            foreach (__reflect_class_constants($this->class) as $c) {
                if ($c['name'] === $this->name) { $this->__m = $c; break; }
            }
            if ($this->__m === null) { $this->__m = ['value' => null, 'public' => true, 'protected' => false, 'private' => false]; }
        }
        return $this->__m;
    }
    public function getName() { return $this->name; }
    public function getValue() { return $this->meta()['value']; }
    public function isPublic() { return $this->meta()['public']; }
    public function isProtected() { return $this->meta()['protected']; }
    public function isPrivate() { return $this->meta()['private']; }
    public function isFinal() { return false; }
    public function isEnumCase() { return false; }
    public function getDeclaringClass() { return new ReflectionClass($this->class); }
}
"#;
