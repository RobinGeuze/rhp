//! The `spl_autoload_register` registry (ext/spl/php_spl.c). Each entry is
//! the resolved callable (php's autoload_func_info): registering an equal
//! one again is a no-op, unregistering removes the equal one, and a loader
//! runs as resolved when it was registered, whatever scope triggers the
//! autoload.

use std::rc::Rc;

use rhp_values::{PhpArray, Thrown, Value};

use crate::calls::Callee;
use crate::{deref, Vm};

/// One registered autoloader.
#[derive(Clone)]
pub(crate) struct Autoloader {
    pub(crate) callee: Callee,
    /// The Closure or invokable object it was given as (alfi->closure).
    closure: Option<Value>,
}

/// autoload_func_info_equals.
fn same(a: &Autoloader, b: &Autoloader) -> bool {
    let same_obj =
        |x: &Option<rhp_values::ObjectHandle>, y: &Option<rhp_values::ObjectHandle>| match (x, y) {
            (Some(x), Some(y)) => Rc::ptr_eq(x, y),
            (None, None) => true,
            _ => false,
        };
    let same_closure = match (&a.closure, &b.closure) {
        (Some(Value::Closure(x)), Some(Value::Closure(y))) => Rc::ptr_eq(x, y),
        (Some(Value::Object(x)), Some(Value::Object(y))) => Rc::ptr_eq(x, y),
        (None, None) => true,
        _ => false,
    };
    same_closure
        && match (&a.callee, &b.callee) {
            (Callee::Closure(x), Callee::Closure(y)) => Rc::ptr_eq(x, y),
            (Callee::Function(f), Callee::Function(g)) => Rc::ptr_eq(&f.proto, &g.proto),
            (Callee::Builtin(x), Callee::Builtin(y)) => x.eq_ignore_ascii_case(y),
            (Callee::Method(m, t, c), Callee::Method(n, u, d)) => {
                Rc::ptr_eq(&m.proto, &n.proto)
                    && m.defining_class == n.defining_class
                    && same_obj(t, u)
                    && c == d
            }
            // Two trampolines: the same name (case-sensitive).
            (Callee::Magic(_, t, c, x), Callee::Magic(_, u, d, y)) => {
                x == y && same_obj(t, u) && c == d
            }
            (Callee::EnumStatic(c, x), Callee::EnumStatic(d, y)) => c == d && x == y,
            _ => false,
        }
}

impl Vm {
    /// The callable `cb` (checked by zpp) as an autoloader entry.
    fn autoloader_of(&self, cb: &Value) -> Option<Autoloader> {
        let callee = match &self.zpp_fcc {
            Some((p, c)) if std::ptr::eq(*p, cb) => c.clone(),
            _ => self.resolve_callable(cb).ok()?,
        };
        let closure = match deref(cb) {
            v @ (Value::Closure(_) | Value::Object(_)) => Some(v),
            _ => None,
        };
        Some(Autoloader { callee, closure })
    }

    /// `spl_autoload_register($cb, prepend: $prepend)`.
    pub(crate) fn register_autoloader_value(
        &mut self,
        cb: &Value,
        prepend: bool,
    ) -> Result<(), Thrown> {
        let Some(a) = self.autoloader_of(cb) else {
            return Ok(());
        };
        if matches!(&a.callee, Callee::Builtin(n) if n.eq_ignore_ascii_case("spl_autoload_call")) {
            return Err(Thrown::error(
                "ValueError",
                "spl_autoload_register(): Argument #1 ($callback) must not be the spl_autoload_call() function",
            ));
        }
        if self.autoloaders.iter().any(|b| same(b, &a)) {
            return Ok(());
        }
        if prepend {
            self.autoloaders.insert(0, a);
        } else {
            self.autoloaders.push(a);
        }
        Ok(())
    }

    /// `spl_autoload_unregister($cb)`: whether an equal entry was removed.
    pub(crate) fn unregister_autoloader_value(&mut self, cb: &Value) -> Result<bool, Thrown> {
        let Some(a) = self.autoloader_of(cb) else {
            return Ok(false);
        };
        if matches!(&a.callee, Callee::Builtin(n) if n.eq_ignore_ascii_case("spl_autoload_call")) {
            rhp_builtins::Host::emit_diagnostic(
                self,
                rhp_builtins::E_DEPRECATED,
                b"spl_autoload_unregister(): Using spl_autoload_call() as a callback for spl_autoload_unregister() is deprecated, to remove all registered autoloaders, call spl_autoload_unregister() for all values returned from spl_autoload_functions()",
            )?;
            self.autoloaders.clear();
            return Ok(true);
        }
        match self.autoloaders.iter().position(|b| same(b, &a)) {
            Some(i) => {
                self.autoloaders.remove(i);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// `spl_autoload_functions()`: a closure or invokable object as given,
    /// a method as `[object or class name, method name]`, a function by
    /// name.
    pub(crate) fn autoload_function_values(&self) -> Vec<Value> {
        let pair = |vm: &Vm, this: &Option<rhp_values::ObjectHandle>, cls, name: &[u8]| {
            let mut a = PhpArray::new();
            a.push(match this {
                Some(h) => Value::Object(h.clone()),
                None => Value::Str(vm.class(cls).name.clone()),
            });
            a.push(Value::str(name.to_vec()));
            Value::Array(a)
        };
        self.autoloaders
            .iter()
            .map(|a| {
                if let Some(c) = &a.closure {
                    return c.clone();
                }
                match &a.callee {
                    Callee::Closure(cd) => Value::Closure(cd.clone()),
                    Callee::Function(f) => Value::Str(f.proto.name.clone()),
                    Callee::Builtin(n) => Value::str(n.as_bytes().to_vec()),
                    Callee::Method(m, this, called) => {
                        pair(self, this, *called, m.proto.name.as_bytes())
                    }
                    Callee::Magic(_, this, called, name) => pair(self, this, *called, name),
                    Callee::EnumStatic(cls, lc) => {
                        let name: &[u8] = match &lc[..] {
                            b"from" => b"from",
                            b"tryfrom" => b"tryFrom",
                            _ => b"cases",
                        };
                        pair(self, &None, *cls, name)
                    }
                }
            })
            .collect()
    }

    /// Where the walk goes on after the loader at `i` ran: past it, or to
    /// what took its place when it unregistered itself.
    pub(crate) fn autoloader_next(&self, i: usize, ran: &Autoloader) -> usize {
        match self.autoloaders.iter().position(|b| same(b, ran)) {
            Some(j) => j + 1,
            None => i,
        }
    }

    /// Run one autoloader for `class` (spl_perform_autoload's
    /// zend_call_known_function).
    pub(crate) fn run_autoloader(&mut self, a: Autoloader, class: Value) -> Result<(), Thrown> {
        if let Some(c) = a.closure {
            return self.call_callable_value(&c, vec![class]).map(|_| ());
        }
        let args = self.callee_by_ref_args(&a.callee, vec![class])?;
        self.run_callee(a.callee, args, None, false).map(|_| ())
    }
}
