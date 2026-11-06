//! The by-name view of a scope's variables (php's symbol table, as
//! zend_rebuild_symbol_table builds it): what extract(), compact() and
//! get_defined_vars() see. A function's variables are its compiled slots
//! (CVs, in compile order) followed by the variables created by name
//! (`Frame::dyn_vars`); the global scope's is the global symbol table.
//! Frames that never use it pay nothing.

use std::cell::RefCell;
use std::rc::Rc;

use rhp_builtins::ScopeVar;
use rhp_bytecode::{FunctionProto, Reg};
use rhp_values::{ArrayKey, PhpArray, Thrown, Value};

use crate::dispatch::cell_define;
use crate::{deref, Vm};

/// The CV of `proto` that the variable `name` is, by name. `$this` and the
/// auto-globals are not variables of the scope (php compiles them to their
/// own fetches), and an empty name is no variable.
pub(crate) fn named_cv(proto: &FunctionProto, name: &[u8]) -> Option<Reg> {
    if name.is_empty() || name == b"this" {
        return None;
    }
    let i = proto
        .cv_var_names
        .iter()
        .take(proto.num_cvs as usize)
        .position(|n| n.as_bytes() == name)? as Reg;
    if proto.superglobals.iter().any(|(r, _)| *r == i) {
        return None;
    }
    Some(i)
}

impl Vm {
    /// The frame whose scope a builtin sees: the innermost user-code frame
    /// (zend_rebuild_symbol_table's "last called user function").
    pub(crate) fn scope_top(&self) -> Option<usize> {
        (0..self.stack.len())
            .rev()
            .find(|&i| !self.is_internal_frame(i))
    }

    /// The frame that owns `top`'s variables: an include or eval frame runs
    /// in its includer's scope.
    fn scope_owner(&self, mut top: usize) -> usize {
        while !self.stack[top].global_scope {
            match self.stack[top].include_caller {
                Some(c) => top = c,
                None => break,
            }
        }
        top
    }

    /// get_defined_vars(): a copy of the scope's variables, undefined ones
    /// left out. A variable that is a PHP reference something else shares
    /// stays one in the copy (zend_array_dup).
    pub(crate) fn scope_vars(&self) -> PhpArray {
        let Some(top) = self.scope_top() else {
            return PhpArray::new();
        };
        let top = self.scope_owner(top);
        let f = &self.stack[top];
        if f.global_scope {
            return match self.globals_array() {
                Value::Array(a) => a,
                _ => PhpArray::new(),
            };
        }
        let mut out = PhpArray::new();
        let proto = &f.proto;
        for (i, name) in proto
            .cv_var_names
            .iter()
            .enumerate()
            .take(proto.num_cvs as usize)
        {
            if named_cv(proto, name.as_bytes()) != Some(i as Reg) {
                continue;
            }
            let v = match &f.regs[i] {
                Value::Undef => continue,
                Value::Ref(rc) => {
                    if rc.borrow().is_undef() {
                        continue;
                    }
                    shared_or_value(rc)
                }
                v => v.clone(),
            };
            out.insert(ArrayKey::from_bytes(name.as_bytes()), v);
        }
        for (name, cell) in &f.dyn_vars {
            if cell.borrow().is_undef() {
                continue;
            }
            out.insert(ArrayKey::from_bytes(name), shared_or_value(cell));
        }
        out
    }

    /// The variable `name` of the builtin's scope, dereferenced.
    pub(crate) fn scope_var(&self, name: &[u8]) -> ScopeVar {
        let Some(top) = self.scope_top() else {
            return ScopeVar::Missing;
        };
        // (Read without copying first: a copy of `Undef` is null.)
        let get = |v: &Value| match v {
            Value::Ref(rc) if rc.borrow().is_undef() => ScopeVar::Undef,
            Value::Undef => ScopeVar::Undef,
            v => ScopeVar::Set(deref(v)),
        };
        match self.var_var_lookup(top, name) {
            None => ScopeVar::Missing,
            Some(Ok(cv)) => get(&self.stack[top].regs[cv as usize]),
            Some(Err(cell)) => get(&cell.borrow()),
        }
    }

    /// Assign `value` to the variable `name` of the builtin's scope (created
    /// when missing), through a reference it holds.
    pub(crate) fn scope_assign(&mut self, name: &[u8], value: Value) -> Result<(), Thrown> {
        let Some(top) = self.scope_top() else {
            return Ok(());
        };
        let old = match self.var_var_slot(top, name) {
            Ok(cv) => match &mut self.stack[top].regs[cv as usize] {
                Value::Ref(rc) => std::mem::replace(&mut *rc.borrow_mut(), value),
                slot => std::mem::replace(slot, value),
            },
            Err(cell) => std::mem::replace(&mut *cell.borrow_mut(), value),
        };
        if matches!(old, Value::Object(_) | Value::Array(_)) {
            self.dtor_check = true;
        }
        Ok(())
    }

    /// Make the variable `name` of the builtin's scope an alias of `cell`.
    pub(crate) fn scope_bind(&mut self, name: &[u8], cell: Rc<RefCell<Value>>) {
        let Some(top) = self.scope_top() else {
            return;
        };
        if self.stack[self.scope_owner(top)].global_scope {
            self.ref_globals.insert(name.to_vec());
        }
        let old = match self.var_var_lookup(top, name) {
            Some(Ok(cv)) => deref(&self.stack[top].regs[cv as usize]),
            Some(Err(c)) => deref(&c.borrow()),
            None => Value::Null,
        };
        self.var_var_bind(top, name, cell);
        if matches!(old, Value::Object(_) | Value::Array(_)) {
            self.dtor_check = true;
        }
    }

    // The variable-variable instructions. Rare in hot code: kept out of
    // the dispatch loop.

    /// `global $$n`.
    #[cold]
    #[inline(never)]
    pub(crate) fn bind_global_dyn(&mut self, top: usize, name: Reg) -> Result<(), Thrown> {
        let gname = self.var_name(top, name, false)?;
        let cell = self.global_cell(&gname);
        cell_define(&cell);
        self.ref_globals.insert(gname.clone());
        self.var_var_bind(top, &gname, cell);
        Ok(())
    }

    /// `$$n = &<src>`.
    #[cold]
    #[inline(never)]
    pub(crate) fn bind_var_var_ref(
        &mut self,
        top: usize,
        name: Reg,
        src: Reg,
    ) -> Result<(), Thrown> {
        let vname = self.var_name(top, name, false)?;
        let cell = match &self.stack[top].regs[src as usize] {
            Value::Ref(rc) => rc.clone(),
            // a by-value call result: assigned
            v => {
                let v = v.clone();
                match self.var_var_slot(top, &vname) {
                    Ok(cv) => self.cv_assign(top, cv, v),
                    Err(cell) => {
                        let mut c = cell.borrow_mut();
                        match &mut *c {
                            Value::Ref(rc) => *rc.borrow_mut() = v,
                            slot => *slot = v,
                        }
                    }
                }
                return Ok(());
            }
        };
        self.var_var_bind(top, &vname, cell);
        Ok(())
    }

    /// `unset($$n)`.
    #[cold]
    #[inline(never)]
    pub(crate) fn unset_var_var(&mut self, top: usize, name: Reg) -> Result<(), Thrown> {
        let vname = self.var_name(top, name, false)?;
        self.var_var_unset(top, &vname);
        Ok(())
    }

    /// `$$n` read: "Undefined variable" when it does not exist, unless
    /// `quiet` (isset, empty, `??`).
    #[cold]
    #[inline(never)]
    pub(crate) fn fetch_var_var(
        &mut self,
        top: usize,
        dst: Reg,
        name: Reg,
        quiet: bool,
    ) -> Result<(), Thrown> {
        let vname = self.var_name(top, name, quiet)?;
        // A read (not isset) of `$$n` with `$n = 'this'` is `$this`
        // (zend_fetch_var's BP_VAR_R special case).
        if !quiet && vname == b"this" {
            if let Some(cv) = self.stack[top].proto.this_cv {
                if let v @ Value::Object(_) = self.reg_get(top, cv) {
                    self.reg_set(top, dst, v);
                    return Ok(());
                }
            }
        }
        let v = match self.var_var_lookup(top, &vname) {
            Some(Ok(cv)) if quiet => self.reg_get(top, cv),
            Some(Ok(cv)) => self.reg_r(top, cv)?,
            Some(Err(cell)) if quiet || !cell.borrow().is_undef() => deref(&cell.borrow()),
            None if quiet => Value::Null,
            _ => {
                let msg = [b"Undefined variable $".as_slice(), &vname].concat();
                self.diag(rhp_builtins::E_WARNING, &msg)?;
                Value::Null
            }
        };
        self.reg_set(top, dst, v);
        Ok(())
    }

    /// `$this` of the builtin's scope (compact('this')).
    pub(crate) fn scope_this(&self) -> Option<Value> {
        let top = self.scope_top()?;
        let f = &self.stack[top];
        let cv = f.proto.this_cv?;
        match f.regs.get(cv as usize)? {
            v @ Value::Object(_) => Some(v.clone()),
            _ => None,
        }
    }
}

/// A symbol-table entry as zend_array_dup copies it: a reference nothing
/// else holds becomes its value.
fn shared_or_value(rc: &Rc<RefCell<Value>>) -> Value {
    if Rc::strong_count(rc) > 1 {
        Value::Ref(rc.clone())
    } else {
        deref(&rc.borrow())
    }
}
