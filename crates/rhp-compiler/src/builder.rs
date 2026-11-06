//! `ProtoBuilder`: one function body under construction, its registers, pools and line table.

use std::collections::HashMap;
use std::rc::Rc;

use crate::parse::cst::{Expression, Identifier};
use rhp_bytecode::{CIdx, FunctionProto, Instr, NIdx, ProtoFlags, Reg};
use rhp_values::{PhpStr, Thrown, Value};

use crate::file::FileCtx;
use crate::fold::const_eq;
use crate::names::NameKind;
use crate::util::{compile_error, is_superglobal};

/// What kind of scope a proto is, for the `__FUNCTION__`/`__METHOD__` magic
/// constants (which read differently at top level vs in a function/method/closure).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScopeKind {
    Main,
    Function,
    Method,
    Closure,
    Init,
}

pub(crate) struct ProtoBuilder<'c> {
    /// The file this body belongs to.
    pub(crate) cx: &'c FileCtx<'c>,
    pub(crate) name: PhpStr,
    /// Enclosing class/trait/enum name for `__CLASS__`/`__METHOD__` (None outside).
    pub(crate) class_name: Option<PhpStr>,
    /// `function &f()` — this proto returns by reference (affects `return`).
    pub(crate) returns_ref: bool,
    /// The scope discriminator for `__FUNCTION__`/`__METHOD__`.
    pub(crate) scope_kind: ScopeKind,
    /// For a closure, PHP 8.5's generated name `{closure:<scope>:<line>}` used by
    /// `__FUNCTION__`/`__METHOD__` inside it (the proto name stays "{closure}").
    pub(crate) magic_fn_name: Option<PhpStr>,
    pub(crate) code: Vec<Instr>,
    pub(crate) consts: Vec<Value>,
    pub(crate) names: Vec<PhpStr>,
    pub(crate) cv_map: Vec<Vec<u8>>, // index = Reg, value = variable name bytes
    pub(crate) exception_table: Vec<rhp_bytecode::ExRegion>,
    /// Per enclosing try-with-finally: positions of pending-return jump
    /// placeholders to patch to the finally entry.
    pub(crate) finally_rets: Vec<Vec<usize>>,
    pub(crate) num_cvs: u16,
    pub(crate) next_temp: Reg,
    pub(crate) high_water: Reg,
    /// The assignment being compiled is a statement of its own: its value
    /// is not used.
    pub(crate) result_unused: bool,
    /// See [`FunctionProto::const_attrs`] / [`FunctionProto::discard_calls`].
    pub(crate) const_attrs: Vec<Rc<Vec<rhp_bytecode::AttributeProto>>>,
    pub(crate) discard_calls: Vec<rhp_bytecode::Pc>,
    /// The function call about to be compiled is an expression statement.
    pub(crate) call_result_unused: bool,
    /// Temporaries below this are protected from `reset_temps` (e.g. a `switch`
    /// subject held live across fall-through case bodies).
    pub(crate) temp_floor: Reg,
    /// Stack of enclosing loops/switches, for `break`/`continue` resolution.
    pub(crate) loops: Vec<LoopCtx>,
    /// Current source line (set per statement); recorded into `lines` on change.
    pub(crate) cur_line: u32,
    /// (pc, line) pairs, appended whenever `cur_line` changes at an emit point.
    pub(crate) lines: Vec<(rhp_bytecode::Pc, u32)>,
    /// Lines of the enclosing call / `new` expressions (innermost last): php
    /// gives the call opcode the line of the callee name (or `new`), not the
    /// statement's, which is what getLine() and traces report.
    pub(crate) call_lines: Vec<u32>,
    /// Set once the CV pre-pass is done: every `$var` of the body must already
    /// have its slot (a later slot would alias a temporary).
    pub(crate) cvs_frozen: bool,
    /// The body refers to `$this` (php's ZEND_ACC_USES_THIS).
    pub(crate) uses_this: bool,
    /// The enclosing loop/switch and finally-body scopes (unique ids; finally
    /// bodies have the high bit set), for checking `goto` jumps.
    pub(crate) goto_scope: Vec<u32>,
    pub(crate) goto_serial: u32,
    /// `label:` name -> (pc, scope path at the label).
    pub(crate) labels: HashMap<Vec<u8>, (u32, Vec<u32>)>,
    /// Pending `goto`s: placeholder position, label, scope path, line.
    pub(crate) gotos: Vec<(usize, Vec<u8>, Vec<u32>, u32)>,
    /// In a property hook: the property's name and the hook (php's
    /// `active_property_info_name` / `active_property_hook_kind`).
    pub(crate) hook: Option<(Vec<u8>, rhp_bytecode::HookKind)>,
    /// The declared return type, for zend_emit_return_type_check's
    /// compile-time checks.
    pub(crate) ret_type: Option<rhp_bytecode::TypeHint>,
    /// The op_array's static_variables: `static` names and closure bindings.
    pub(crate) static_vars: Vec<Vec<u8>>,
    /// Compiling the by-value side of a SEND_FUNC_ARG argument: `[]` there
    /// is a runtime Error, not a compile error.
    pub(crate) func_arg_read: bool,
    /// Nullsafe short-circuiting (zend_short_circuiting_commit): the object
    /// node of the chain link being compiled, which continues its chain...
    pub(crate) sc_inner: usize,
    /// ...and the pending `JmpNull` guards of the open chain, jumping to
    /// where its outermost link's result is set to null.
    pub(crate) sc_jumps: Vec<(usize, Reg)>,
    /// Compiling a `const` declaration's value (a constant expression).
    pub(crate) in_const_decl: bool,
    /// CV 0 is `$this`, always set (instance methods and hooks).
    pub(crate) this_bound: bool,
    /// The body contains `yield`.
    pub(crate) is_generator: bool,
}

/// A break/continue target under construction. Jump placeholders are recorded
/// as code positions and patched once the loop's exit/continue labels are known.
#[derive(Default)]
pub(crate) struct LoopCtx {
    /// `finally_rets.len()` when the loop was entered: a break/continue
    /// inside more try-with-finally levels than this must run them.
    pub(crate) finally_depth: usize,
    /// Finally bodies around the loop: a jump from a deeper one leaves a
    /// finally block.
    pub(crate) finally_bodies: usize,
    pub(crate) is_switch: bool,
    pub(crate) break_jumps: Vec<usize>,
    pub(crate) continue_jumps: Vec<usize>,
    /// A temp the loop frees when it ends (FE_FREE of a by-reference
    /// foreach over `$a['k']` / `$o->p`): a `break N` / `continue N`
    /// leaving it frees it too.
    pub(crate) free: Option<Reg>,
}

impl<'c> ProtoBuilder<'c> {
    pub(crate) fn new(cx: &'c FileCtx<'c>, name: PhpStr) -> Self {
        ProtoBuilder {
            cx,
            name,
            class_name: None,
            returns_ref: false,
            scope_kind: ScopeKind::Function,
            magic_fn_name: None,
            code: Vec::new(),
            consts: Vec::new(),
            names: Vec::new(),
            cv_map: Vec::new(),
            exception_table: Vec::new(),
            finally_rets: Vec::new(),
            num_cvs: 0,
            next_temp: 0,
            high_water: 0,
            result_unused: false,
            const_attrs: Vec::new(),
            discard_calls: Vec::new(),
            call_result_unused: false,
            temp_floor: 0,
            loops: Vec::new(),
            cur_line: 0,
            lines: Vec::new(),
            call_lines: Vec::new(),
            cvs_frozen: false,
            uses_this: false,
            goto_scope: Vec::new(),
            goto_serial: 0,
            labels: HashMap::new(),
            gotos: Vec::new(),
            hook: None,
            ret_type: None,
            this_bound: false,
            in_const_decl: false,
            sc_inner: 0,
            sc_jumps: Vec::new(),
            func_arg_read: false,
            static_vars: Vec::new(),
            is_generator: false,
        }
    }

    /// Enter a loop/switch (`finally == false`) or finally-body scope.
    /// A new loop (or switch) level for break/continue.
    pub(crate) fn loop_ctx(&self, is_switch: bool) -> LoopCtx {
        LoopCtx {
            finally_depth: self.finally_rets.len(),
            finally_bodies: self.finally_bodies(),
            is_switch,
            ..LoopCtx::default()
        }
    }

    /// The finally bodies being compiled.
    pub(crate) fn finally_bodies(&self) -> usize {
        self.goto_scope
            .iter()
            .filter(|s| *s & (1 << 31) != 0)
            .count()
    }

    pub(crate) fn enter_goto_scope(&mut self, finally: bool) {
        self.goto_serial += 1;
        let id = self.goto_serial | if finally { 1 << 31 } else { 0 };
        self.goto_scope.push(id);
    }

    /// Bind each `goto` to its label (php's zend_resolve_goto_label plus the
    /// finally checks of pass_two): the label must exist, must not be inside a
    /// loop/switch the `goto` is not in, and the jump may not cross a finally
    /// body boundary. A jump out of a try/catch with a finally runs it first.
    fn resolve_gotos(&mut self) -> Result<(), Thrown> {
        for (pos, name, scope, line) in std::mem::take(&mut self.gotos) {
            let label = String::from_utf8_lossy(&name).into_owned();
            let Some((target, lscope)) = self.labels.get(&name) else {
                return Err(compile_error(
                    format!("'goto' to undefined label '{label}'"),
                    line,
                ));
            };
            let common = scope
                .iter()
                .zip(lscope.iter())
                .take_while(|(a, b)| a == b)
                .count();
            if lscope[common..].iter().any(|id| id & (1 << 31) != 0) {
                return Err(compile_error(
                    "jump into a finally block is disallowed",
                    line,
                ));
            }
            if scope[common..].iter().any(|id| id & (1 << 31) != 0) {
                return Err(compile_error(
                    "jump out of a finally block is disallowed",
                    line,
                ));
            }
            if common < lscope.len() {
                return Err(compile_error(
                    "'goto' into loop or switch statement is disallowed",
                    line,
                ));
            }
            let target = *target;
            self.code[pos] = match self.code[pos] {
                Instr::JmpFinally { .. } => Instr::JmpFinally { target },
                _ => Instr::Jmp { target },
            };
        }
        Ok(())
    }

    pub(crate) fn finish(mut self) -> Result<FunctionProto, Thrown> {
        self.resolve_gotos()?;
        let mut proto = FunctionProto::new(self.name);
        proto.num_cvs = self.num_cvs;
        proto.num_registers = self.high_water.max(self.num_cvs);
        proto.code = self.code;
        proto.consts = self.consts;
        proto.names = self.names;
        proto.exception_table = self.exception_table;
        proto.const_attrs = self.const_attrs;
        proto.discard_calls = self.discard_calls;
        proto.discard_calls.shrink_to_fit();
        proto.cv_var_names = self
            .cv_map
            .iter()
            .map(|n| PhpStr::from_bytes(n.clone()))
            .collect();
        proto.debug.lines = self.lines;
        proto.debug.file = self.cx.file.clone();
        proto.superglobals = self
            .cv_map
            .iter()
            .enumerate()
            .filter(|(_, n)| is_superglobal(n))
            .map(|(i, n)| (i as Reg, PhpStr::from_bytes(n.clone())))
            .collect();
        proto.strict_types = self.cx.strict_types;
        proto.returns_ref = self.returns_ref;
        if self.cx.is_prelude {
            proto.flags.insert(ProtoFlags::INTERNAL);
        }
        if self.uses_this {
            proto.flags.insert(ProtoFlags::USES_THIS);
        }
        // The VM keeps the proto as it is for as long as the program lives
        // (with opcache, across requests): drop the growth slack.
        proto.code.shrink_to_fit();
        proto.consts.shrink_to_fit();
        proto.names.shrink_to_fit();
        proto.exception_table.shrink_to_fit();
        proto.debug.lines.shrink_to_fit();
        proto.superglobals.shrink_to_fit();
        Ok(proto)
    }

    pub(crate) fn emit(&mut self, instr: Instr) {
        let line = match (&instr, self.call_lines.last()) {
            (
                Instr::Call { .. }
                | Instr::CallV { .. }
                | Instr::CallDyn { .. }
                | Instr::CallDynV { .. }
                | Instr::New { .. }
                | Instr::NewV { .. }
                | Instr::CallMethod { .. }
                | Instr::CallMethodV { .. }
                | Instr::CallMethodDyn { .. }
                | Instr::CallStatic { .. }
                | Instr::CallParentHook { .. }
                | Instr::CallStaticV { .. }
                | Instr::CallStaticDyn { .. }
                | Instr::CallMethodDynV { .. }
                | Instr::CallStaticDynV { .. },
                Some(&l),
            ) if l != 0 => l,
            _ => self.cur_line,
        };
        // Record a line entry whenever the source line changes (for debug info).
        if self.lines.last().map(|(_, l)| *l) != Some(line) {
            self.lines.push((self.code.len() as rhp_bytecode::Pc, line));
        }
        self.code.push(instr);
    }

    pub(crate) fn alloc_temp(&mut self) -> Reg {
        let r = self.next_temp;
        self.next_temp += 1;
        self.high_water = self.high_water.max(self.next_temp);
        r
    }

    /// Allocate `n` contiguous temporaries (for a call arg window).
    pub(crate) fn alloc_temps(&mut self, n: u16) -> Reg {
        let base = self.next_temp;
        self.next_temp += n;
        self.high_water = self.high_water.max(self.next_temp);
        base
    }

    /// Free temporaries back to the current floor (end of a statement).
    pub(crate) fn reset_temps(&mut self) {
        self.next_temp = self.temp_floor;
    }

    /// Patch a placeholder `Nop` at `pos` into a jump to `target`.
    pub(crate) fn patch_jmp(&mut self, pos: usize, target: u32) {
        self.code[pos] = match self.code[pos] {
            Instr::JmpFinally { .. } => Instr::JmpFinally { target },
            _ => Instr::Jmp { target },
        };
    }

    pub(crate) fn here(&self) -> u32 {
        self.code.len() as u32
    }

    /// php's this_guaranteed_exists: an instance method (or hook), whose
    /// CV 0 always holds the object.
    pub(crate) fn this_guaranteed(&self) -> bool {
        self.this_bound
    }

    /// The register of `$this` for a read (ZEND_FETCH_THIS): checked
    /// unless the object is guaranteed.
    pub(crate) fn this_reg(&mut self) -> Reg {
        let cv = self.cv(b"this");
        if !self.this_guaranteed() {
            self.emit(Instr::CheckThis { cv });
        }
        cv
    }

    pub(crate) fn cv(&mut self, var_name: &[u8]) -> Reg {
        if self.cvs_frozen && var_name == b"this" {
            self.uses_this = true;
        }
        if let Some(i) = self.cv_map.iter().position(|n| n == var_name) {
            return i as Reg;
        }
        debug_assert!(
            !self.cvs_frozen,
            "CV ${} was missed by the pre-pass (its slot would alias a temporary)",
            String::from_utf8_lossy(var_name)
        );
        let r = self.cv_map.len() as Reg;
        self.cv_map.push(var_name.to_vec());
        r
    }

    pub(crate) fn intern_name(&mut self, bytes: &[u8]) -> NIdx {
        if let Some(i) = self.names.iter().position(|n| n.as_bytes() == bytes) {
            return i as NIdx;
        }
        let i = self.names.len() as NIdx;
        self.names.push(PhpStr::from_bytes(bytes.to_vec()));
        i
    }

    /// Intern an optional global-fallback name, or `NO_NAME` when there is none.
    pub(crate) fn intern_fallback(&mut self, fb: &Option<Vec<u8>>) -> NIdx {
        match fb {
            Some(f) => self.intern_name(f),
            None => rhp_bytecode::NO_NAME,
        }
    }

    /// The register holding the unresolved callee `name` (then `fallback`)
    /// of the call being compiled, emitting its `ResolveFunc` the first time.
    pub(crate) fn resolve_callee(
        &mut self,
        callee: &mut Option<Reg>,
        name: &[u8],
        fallback: &Option<Vec<u8>>,
    ) -> Reg {
        if let Some(r) = *callee {
            return r;
        }
        let dst = self.alloc_temp();
        let name = self.intern_name(&name.to_ascii_lowercase());
        let fallback = self.intern_fallback(&fallback.as_ref().map(|f| f.to_ascii_lowercase()));
        self.emit(Instr::ResolveFunc {
            dst,
            name,
            fallback,
        });
        *callee = Some(dst);
        dst
    }

    /// Resolve a first-class-callable's target function to a single name. An
    /// unqualified reference inside a namespace prefers the namespaced user
    /// function when one is declared, else the global name (builtins are
    /// global), returned with the namespaced name it fell back from.
    pub(crate) fn resolve_callable_name(&self, id: &Identifier) -> (Vec<u8>, Option<Vec<u8>>) {
        let (primary, fallback) = self.cx.resolve_name(id, NameKind::Func);
        match fallback {
            None => (primary, None),
            Some(global) => {
                if self.cx.sigs.contains_key(&primary.to_ascii_lowercase()) {
                    (primary, None)
                } else {
                    (global, Some(primary))
                }
            }
        }
    }

    /// Emit prologue code for non-constant parameter defaults (e.g.
    /// `$v = self::CONST`): for each, `JmpIfArg index -> after` skips the init when
    /// the argument was passed, else the default expression is evaluated into the
    /// param's CV (in the callee's scope, so `self::`/`static::` resolve).
    /// A typed parameter's default is then checked against its type
    /// (php's RECV_INIT), at the parameter's line.
    pub(crate) fn emit_param_defaults(
        &mut self,
        defs: &[(u16, Reg, &Expression, Option<u32>)],
    ) -> Result<(), Thrown> {
        for &(index, cv, expr, check_line) in defs {
            let jpos = self.code.len();
            self.emit(Instr::Nop); // JmpIfArg index -> after (patched)
            self.in_const_decl = true;
            let t = self.compile_expr(expr);
            self.in_const_decl = false;
            let t = t?;
            self.emit(Instr::Mov { dst: cv, src: t });
            if let Some(line) = check_line {
                let saved = std::mem::replace(&mut self.cur_line, line);
                self.emit(Instr::RecvInitCheck { index });
                self.cur_line = saved;
            }
            let after = self.here();
            self.code[jpos] = Instr::JmpIfArg {
                index,
                target: after,
            };
            self.reset_temps();
        }
        Ok(())
    }

    pub(crate) fn intern_const(&mut self, v: Value) -> CIdx {
        if let Some(i) = self.consts.iter().position(|c| const_eq(c, &v)) {
            return i as CIdx;
        }
        let i = self.consts.len() as CIdx;
        self.consts.push(v);
        i
    }
}
