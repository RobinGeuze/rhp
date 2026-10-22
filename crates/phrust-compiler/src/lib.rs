//! AST → bytecode compiler (Phase-1 subset).
//!
//! Lowers a `phrust_frontend::ParsedFile` into a `phrust_bytecode::Program`.
//! Owns the **structural refusals** (eval/include/backticks/`(unset)`-cast,
//! variable-variable writes): those nodes never produce an opcode — the
//! compiler returns `Err(Thrown::Unsupported)` at compile time (§4 of the spec).
//!
//! Register model (spec §2.2): a pre-pass assigns one CV register per named
//! `$var` so CVs occupy `[0, num_cvs)`; temporaries bump above and are freed at
//! statement boundaries; `num_registers` is the peak.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use phrust_bytecode::{
    CIdx, ClassProto, ClassRef, CmpKind, ConstInitProto, ConstProto, FnIdx, FunctionProto, Instr,
    MethodProto, NIdx, Program, PropDefaultProto, PropProto, Reg, StaticPropProto,
};
use phrust_frontend::cst::{
    Access, Argument, ArrayElement, AssignmentOperator, BinaryOperator, Class, ClassLikeMember,
    ClassLikeMemberSelector, CompositeString, Construct, Expression, ForBody, ForeachBody,
    ForeachTarget, Hint, Identifier, IfBody, Literal, Match, MatchArm, MethodBody, Modifier,
    Program as AstProgram, Property, PropertyItem, Statement, StringPart, SwitchBody, SwitchCase,
    UnaryPrefixOperator, Variable, WhileBody,
};
use phrust_values::{ClassFlags, ClassKind, Visibility};

/// A step in an lvalue path (compiler-side; key regs are pre-evaluated).
enum CursorStep {
    Key(Reg),
    Append,
    Prop(NIdx),
}

/// Where an lvalue cursor roots.
enum LvalRoot {
    Cv(Reg),
    StaticProp(ClassRef, NIdx),
}

/// The body form of a closure being lowered.
enum ClosureBody<'a> {
    /// Arrow function: a single expression, implicitly `return`ed.
    Arrow(&'a Expression<'a>),
    /// `function(){ ... }`: a block of statements.
    Block(&'a phrust_frontend::cst::Block<'a>),
}
use phrust_values::{ArrayKey, PhpArray, PhpStr, Thrown, Value};

thread_local! {
    /// Sink for anonymous closure / arrow-function protos produced anywhere in
    /// the program. Compilation is single-threaded and non-reentrant per
    /// `compile()` call; the list is cleared on entry and drained into
    /// `Program::closures` on exit. A closure's `fnidx` is its position here.
    static CLOSURES: RefCell<Vec<Rc<FunctionProto>>> = const { RefCell::new(Vec::new()) };
}

/// Compile a parsed program (borrowed from a `phrust_frontend::Arena`) into an
/// owned, runnable bytecode `Program`. No `'arena` borrow escapes.
pub fn compile(program: &AstProgram) -> Result<Program, Thrown> {
    CLOSURES.with(|c| c.borrow_mut().clear());
    // Signature pre-pass: name -> per-parameter by-ref flags, so a call site can
    // marshal by-ref arguments even though the callee is resolved by name.
    let mut sigs: HashMap<Vec<u8>, Vec<bool>> = HashMap::new();
    for stmt in program.statements.iter() {
        if let Statement::Function(func) = stmt {
            let flags: Vec<bool> =
                func.parameter_list.parameters.iter().map(|p| p.ampersand.is_some()).collect();
            sigs.insert(func.name.value.to_ascii_lowercase(), flags);
        }
    }
    let sigs = Rc::new(sigs);

    // Hoist top-level function + class declarations. Start the class list with
    // the built-in exception hierarchy (the "prelude").
    let mut functions = Vec::new();
    let mut classes = prelude_classes()?;
    for stmt in program.statements.iter() {
        match stmt {
            Statement::Function(func) => {
                functions.push(Rc::new(compile_function(func, sigs.clone())?));
            }
            Statement::Class(class) => classes.push(compile_class(class, &sigs)?),
            Statement::Interface(iface) => classes.push(compile_interface(iface)?),
            Statement::Trait(tr) => classes.push(compile_trait(tr, &sigs)?),
            Statement::Enum(en) => classes.push(compile_enum(en, &sigs)?),
            _ => {}
        }
    }

    // Compile the script body ({main}); its CVs are the globals.
    let mut c = ProtoBuilder::new(PhpStr::from("{main}"));
    c.sigs = sigs.clone();
    for stmt in program.statements.iter() {
        c.scan_stmt_vars(stmt);
    }
    c.num_cvs = c.cv_map.len() as u16;
    c.next_temp = c.num_cvs;
    c.high_water = c.num_cvs;
    c.temp_floor = c.num_cvs;

    for stmt in program.statements.iter() {
        c.compile_stmt(stmt)?;
    }
    c.emit(Instr::RetNull);

    let closures = CLOSURES.with(|c| std::mem::take(&mut *c.borrow_mut()));
    Ok(Program { main: Rc::new(c.finish()), functions, classes, closures })
}

// --- OOP: class declaration compilation ------------------------------------

fn modifiers_visibility(mods: &phrust_frontend::cst::Sequence<Modifier>) -> Visibility {
    for m in mods.iter() {
        match m {
            Modifier::Private(_) | Modifier::PrivateSet(_) => return Visibility::Private,
            Modifier::Protected(_) | Modifier::ProtectedSet(_) => return Visibility::Protected,
            Modifier::Public(_) | Modifier::PublicSet(_) => return Visibility::Public,
            _ => {}
        }
    }
    Visibility::Public
}

fn has_modifier(mods: &phrust_frontend::cst::Sequence<Modifier>, want: fn(&Modifier) -> bool) -> bool {
    mods.iter().any(want)
}

fn compile_class(
    class: &Class,
    sigs: &Rc<std::collections::HashMap<Vec<u8>, Vec<bool>>>,
) -> Result<ClassProto, Thrown> {
    let name = PhpStr::from_bytes(class.name.value.to_vec());
    let flags = ClassFlags {
        is_abstract: has_modifier(&class.modifiers, |m| matches!(m, Modifier::Abstract(_))),
        is_final: has_modifier(&class.modifiers, |m| matches!(m, Modifier::Final(_))),
        is_readonly: has_modifier(&class.modifiers, |m| matches!(m, Modifier::Readonly(_))),
    };
    // Single inheritance: a class `extends` one parent.
    let parent = class.extends.as_ref().and_then(|e| {
        e.types.iter().next().map(|id| PhpStr::from_bytes(identifier_name(id).to_vec()))
    });
    let interfaces: Vec<PhpStr> = class
        .implements
        .as_ref()
        .map(|i| i.types.iter().map(|id| PhpStr::from_bytes(identifier_name(id).to_vec())).collect())
        .unwrap_or_default();

    let m = compile_members(&class.members, sigs)?;
    Ok(ClassProto {
        name,
        kind: ClassKind::Class,
        flags,
        parent,
        interfaces,
        traits: m.traits,
        methods: m.methods,
        props: m.props,
        static_props: m.static_props,
        constants: m.constants,
        enum_cases: Vec::new(),
        enum_backing: None,
    })
}

/// Compile an `enum` declaration into a `ClassProto` of kind `Enum`.
///
/// Enum cases are collected and materialized by the VM at link time (one
/// singleton object per case, exposed as class constants). The enum implements
/// `UnitEnum` (pure) or `BackedEnum` (backed), plus any declared interfaces.
fn compile_enum(
    en: &phrust_frontend::cst::Enum,
    sigs: &Rc<std::collections::HashMap<Vec<u8>, Vec<bool>>>,
) -> Result<ClassProto, Thrown> {
    use phrust_bytecode::EnumBacking;
    let name = PhpStr::from_bytes(en.name.value.to_vec());

    let backing = match &en.backing_type_hint {
        None => None,
        Some(h) => match &h.hint {
            Hint::Integer(_) => Some(EnumBacking::Int),
            Hint::String(_) => Some(EnumBacking::Str),
            _ => {
                return Err(Thrown::unsupported(
                    "enum backing type",
                    "only int and string backings are valid",
                ))
            }
        },
    };

    let mut interfaces: Vec<PhpStr> = en
        .implements
        .as_ref()
        .map(|i| i.types.iter().map(|id| PhpStr::from_bytes(identifier_name(id).to_vec())).collect())
        .unwrap_or_default();
    // Every enum implements UnitEnum; a backed enum also implements BackedEnum.
    interfaces.push(PhpStr::from("UnitEnum"));
    if backing.is_some() {
        interfaces.push(PhpStr::from("BackedEnum"));
    }

    let m = compile_members(&en.members, sigs)?;
    Ok(ClassProto {
        name,
        kind: ClassKind::Enum,
        // Enums are implicitly final.
        flags: ClassFlags { is_abstract: false, is_final: true, is_readonly: false },
        parent: None,
        interfaces,
        traits: m.traits,
        methods: m.methods,
        props: m.props,
        static_props: m.static_props,
        constants: m.constants,
        enum_cases: m.enum_cases,
        enum_backing: backing,
    })
}

#[derive(Default)]
struct CompiledMembers {
    methods: Vec<MethodProto>,
    props: Vec<PropProto>,
    static_props: Vec<StaticPropProto>,
    constants: Vec<ConstProto>,
    traits: Vec<PhpStr>,
    enum_cases: Vec<phrust_bytecode::EnumCaseProto>,
}

fn compile_members(
    members: &phrust_frontend::cst::Sequence<ClassLikeMember>,
    sigs: &Rc<std::collections::HashMap<Vec<u8>, Vec<bool>>>,
) -> Result<CompiledMembers, Thrown> {
    let mut out = CompiledMembers::default();
    for member in members.iter() {
        match member {
            ClassLikeMember::Method(m) => {
                let (mp, promoted) = compile_method(m, sigs)?;
                out.methods.push(mp);
                out.props.extend(promoted);
            }
            ClassLikeMember::Property(Property::Plain(p)) => {
                let visibility = modifiers_visibility(&p.modifiers);
                let is_static = has_modifier(&p.modifiers, |m| matches!(m, Modifier::Static(_)));
                let is_readonly = has_modifier(&p.modifiers, |m| matches!(m, Modifier::Readonly(_)));
                for item in p.items.iter() {
                    let (var, default) = match item {
                        PropertyItem::Concrete(c) => {
                            (c.variable.name, PropDefaultProto::Const(const_value_static(c.value)?))
                        }
                        PropertyItem::Abstract(a) => (a.variable.name, PropDefaultProto::None),
                    };
                    let pname = PhpStr::from_bytes(strip_dollar(var).to_vec());
                    if is_static {
                        let init = match &default {
                            PropDefaultProto::Const(v) => ConstInitProto::Const(v.clone()),
                            _ => ConstInitProto::Const(Value::Null),
                        };
                        out.static_props.push(StaticPropProto { name: pname, visibility, is_readonly, init });
                    } else {
                        out.props.push(PropProto { name: pname, visibility, is_readonly, default });
                    }
                }
            }
            ClassLikeMember::Property(Property::Hooked(_)) => {
                return Err(Thrown::unsupported("property hooks", "deferred"))
            }
            ClassLikeMember::Constant(c) => {
                let visibility = modifiers_visibility(&c.modifiers);
                for item in c.items.iter() {
                    out.constants.push(ConstProto {
                        name: PhpStr::from_bytes(item.name.value.to_vec()),
                        visibility,
                        value: ConstInitProto::Const(const_value_static(item.value)?),
                    });
                }
            }
            ClassLikeMember::TraitUse(tu) => {
                for t in tu.trait_names.iter() {
                    out.traits.push(PhpStr::from_bytes(identifier_name(t).to_vec()));
                }
            }
            ClassLikeMember::EnumCase(ec) => {
                use phrust_frontend::cst::EnumCaseItem;
                let (name, value) = match &ec.item {
                    EnumCaseItem::Unit(u) => {
                        (PhpStr::from_bytes(u.name.value.to_vec()), None)
                    }
                    EnumCaseItem::Backed(b) => (
                        PhpStr::from_bytes(b.name.value.to_vec()),
                        Some(const_value_static(b.value)?),
                    ),
                };
                out.enum_cases.push(phrust_bytecode::EnumCaseProto { name, value });
            }
        }
    }
    Ok(out)
}

fn compile_trait(
    t: &phrust_frontend::cst::Trait,
    sigs: &Rc<std::collections::HashMap<Vec<u8>, Vec<bool>>>,
) -> Result<ClassProto, Thrown> {
    let m = compile_members(&t.members, sigs)?;
    Ok(ClassProto {
        name: PhpStr::from_bytes(t.name.value.to_vec()),
        kind: ClassKind::Trait,
        flags: ClassFlags::default(),
        parent: None,
        interfaces: Vec::new(),
        traits: m.traits,
        methods: m.methods,
        props: m.props,
        static_props: m.static_props,
        constants: m.constants,
        enum_cases: Vec::new(),
        enum_backing: None,
    })
}

/// The built-in exception/throwable class hierarchy, as PHP source. Compiled
/// and prepended to every program so `throw`/`catch` and engine errors work.
const PRELUDE: &str = r#"<?php
interface Throwable {}
interface Stringable {}
interface UnitEnum {}
interface BackedEnum extends UnitEnum {}
class Exception implements Throwable {
    protected $message = "";
    protected $code = 0;
    public function __construct($message = "", $code = 0, $previous = null) {
        $this->message = $message;
        $this->code = $code;
    }
    public function getMessage() { return $this->message; }
    public function getCode() { return $this->code; }
    public function getPrevious() { return null; }
}
class Error implements Throwable {
    protected $message = "";
    protected $code = 0;
    public function __construct($message = "", $code = 0, $previous = null) {
        $this->message = $message;
        $this->code = $code;
    }
    public function getMessage() { return $this->message; }
    public function getCode() { return $this->code; }
    public function getPrevious() { return null; }
}
class ErrorException extends Exception {}
class TypeError extends Error {}
class ValueError extends Error {}
class ArithmeticError extends Error {}
class DivisionByZeroError extends ArithmeticError {}
class ArgumentCountError extends TypeError {}
class UnhandledMatchError extends Error {}
class AssertionError extends Error {}
class LogicException extends Exception {}
class InvalidArgumentException extends LogicException {}
class LengthException extends LogicException {}
class DomainException extends LogicException {}
class RuntimeException extends Exception {}
class OutOfBoundsException extends RuntimeException {}
class OutOfRangeException extends LogicException {}
class RangeException extends RuntimeException {}
class OverflowException extends RuntimeException {}
class UnderflowException extends RuntimeException {}
class UnexpectedValueException extends RuntimeException {}
class JsonException extends Exception {}
"#;

fn prelude_classes() -> Result<Vec<ClassProto>, Thrown> {
    let arena = phrust_frontend::Arena::new();
    let parsed = arena.parse(b"<prelude>", PRELUDE.as_bytes());
    let sigs = Rc::new(std::collections::HashMap::new());
    let mut out = Vec::new();
    for stmt in parsed.program().statements.iter() {
        match stmt {
            Statement::Class(c) => out.push(compile_class(c, &sigs)?),
            Statement::Interface(i) => out.push(compile_interface(i)?),
            _ => {}
        }
    }
    Ok(out)
}

fn compile_interface(
    iface: &phrust_frontend::cst::Interface,
) -> Result<ClassProto, Thrown> {
    let name = PhpStr::from_bytes(iface.name.value.to_vec());
    let interfaces: Vec<PhpStr> = iface
        .extends
        .as_ref()
        .map(|e| e.types.iter().map(|id| PhpStr::from_bytes(identifier_name(id).to_vec())).collect())
        .unwrap_or_default();
    let mut constants = Vec::new();
    for member in iface.members.iter() {
        if let ClassLikeMember::Constant(c) = member {
            let visibility = modifiers_visibility(&c.modifiers);
            for item in c.items.iter() {
                constants.push(ConstProto {
                    name: PhpStr::from_bytes(item.name.value.to_vec()),
                    visibility,
                    value: ConstInitProto::Const(const_value_static(item.value)?),
                });
            }
        }
        // Interface method signatures / properties contribute no runtime members.
    }
    Ok(ClassProto {
        name,
        kind: ClassKind::Interface,
        flags: ClassFlags::default(),
        parent: None,
        interfaces,
        traits: Vec::new(),
        methods: Vec::new(),
        props: Vec::new(),
        static_props: Vec::new(),
        constants,
        enum_cases: Vec::new(),
        enum_backing: None,
    })
}

fn compile_method(
    m: &phrust_frontend::cst::Method,
    sigs: &Rc<std::collections::HashMap<Vec<u8>, Vec<bool>>>,
) -> Result<(MethodProto, Vec<PropProto>), Thrown> {
    let name = PhpStr::from_bytes(m.name.value.to_vec());
    let visibility = modifiers_visibility(&m.modifiers);
    let is_static = has_modifier(&m.modifiers, |md| matches!(md, Modifier::Static(_)));
    let is_abstract = has_modifier(&m.modifiers, |md| matches!(md, Modifier::Abstract(_)));
    let is_final = has_modifier(&m.modifiers, |md| matches!(md, Modifier::Final(_)));

    let mut c = ProtoBuilder::new(name.clone());
    c.sigs = sigs.clone();

    // Instance methods reserve CV 0 for `$this`.
    if !is_static {
        let this = c.cv(b"this");
        debug_assert_eq!(this, 0);
    }

    // Constructor property promotion: a param with a modifier declares a property.
    let mut promoted: Vec<PropProto> = Vec::new();
    let mut promo_stores: Vec<(NIdx, Reg)> = Vec::new(); // (prop name, param cv)

    let mut params = Vec::new();
    for p in m.parameter_list.parameters.iter() {
        let var = strip_dollar(p.variable.name);
        let cv = c.cv(var);
        let pname = c.intern_name(var);
        let default = match &p.default_value {
            Some(dv) => Some(c.const_default(dv.value)?),
            None => None,
        };
        if !p.modifiers.is_empty() {
            let vis = modifiers_visibility(&p.modifiers);
            let ro = has_modifier(&p.modifiers, |md| matches!(md, Modifier::Readonly(_)));
            promoted.push(PropProto {
                name: PhpStr::from_bytes(var.to_vec()),
                visibility: vis,
                is_readonly: ro,
                default: PropDefaultProto::None,
            });
            promo_stores.push((c.intern_name(var), cv));
        }
        params.push(phrust_bytecode::ParamInfo {
            name: pname,
            cv,
            by_ref: p.ampersand.is_some(),
            variadic: p.ellipsis.is_some(),
            default,
        });
    }
    let num_params = params.len() as u16;

    let empty: &[Statement] = &[];
    let body_stmts: &[_] = match &m.body {
        MethodBody::Concrete(block) => block.statements.as_slice(),
        MethodBody::Abstract(_) => empty,
    };

    for s in body_stmts {
        c.scan_stmt_vars(s);
    }
    c.num_cvs = c.cv_map.len() as u16;
    c.next_temp = c.num_cvs;
    c.high_water = c.num_cvs;
    c.temp_floor = c.num_cvs;

    // Emit `$this->name = $name` for each promoted param, before the body.
    for (prop_name, param_cv) in &promo_stores {
        c.emit(Instr::LvalCv { cv: 0 });
        c.emit(Instr::LvalProp { name: *prop_name });
        c.emit(Instr::AssignLval { src: *param_cv });
    }

    for s in body_stmts {
        c.compile_stmt(s)?;
    }
    c.emit(Instr::RetNull);

    let mut proto = c.finish();
    proto.num_params = num_params;
    proto.params = params;
    proto.this_cv = if is_static { None } else { Some(0) };
    proto.is_static = is_static;

    let mp = MethodProto { name, proto: Rc::new(proto), visibility, is_static, is_abstract, is_final };
    Ok((mp, promoted))
}

/// Compile one `function name(params) { body }` into a `FunctionProto`.
/// Parameters occupy the first CV registers, in order.
fn compile_function(
    func: &phrust_frontend::cst::Function,
    sigs: Rc<HashMap<Vec<u8>, Vec<bool>>>,
) -> Result<FunctionProto, Thrown> {
    let fname = func.name.value;
    let mut c = ProtoBuilder::new(PhpStr::from_bytes(fname.to_vec()));
    c.sigs = sigs;

    let mut params = Vec::new();
    for p in func.parameter_list.parameters.iter() {
        let var = strip_dollar(p.variable.name);
        let cv = c.cv(var); // params get CV 0,1,2,... in order
        let name = c.intern_name(var);
        let default = match &p.default_value {
            Some(dv) => Some(c.const_default(dv.value)?),
            None => None,
        };
        params.push(phrust_bytecode::ParamInfo {
            name,
            cv,
            by_ref: p.ampersand.is_some(),
            variadic: p.ellipsis.is_some(),
            default,
        });
    }
    let num_params = params.len() as u16;

    // Scan the body for the rest of the function's local variables.
    for s in func.body.statements.iter() {
        c.scan_stmt_vars(s);
    }
    c.num_cvs = c.cv_map.len() as u16;
    c.next_temp = c.num_cvs;
    c.high_water = c.num_cvs;
    c.temp_floor = c.num_cvs;

    for s in func.body.statements.iter() {
        c.compile_stmt(s)?;
    }
    c.emit(Instr::RetNull);

    let mut proto = c.finish();
    proto.num_params = num_params;
    proto.params = params;
    Ok(proto)
}

struct ProtoBuilder {
    name: PhpStr,
    code: Vec<Instr>,
    consts: Vec<Value>,
    names: Vec<PhpStr>,
    cv_map: Vec<Vec<u8>>, // index = Reg, value = variable name bytes
    caches: Vec<phrust_bytecode::InlineCache>,
    exception_table: Vec<phrust_bytecode::ExRegion>,
    /// Per enclosing try-with-finally: positions of pending-return jump
    /// placeholders to patch to the finally entry.
    finally_rets: Vec<Vec<usize>>,
    num_cvs: u16,
    next_temp: Reg,
    high_water: Reg,
    /// Temporaries below this are protected from `reset_temps` (e.g. a `switch`
    /// subject held live across fall-through case bodies).
    temp_floor: Reg,
    /// Stack of enclosing loops/switches, for `break`/`continue` resolution.
    loops: Vec<LoopCtx>,
    /// Function name (lowercase) -> per-parameter by-ref flags (for call sites).
    sigs: Rc<HashMap<Vec<u8>, Vec<bool>>>,
}

/// A break/continue target under construction. Jump placeholders are recorded
/// as code positions and patched once the loop's exit/continue labels are known.
#[derive(Default)]
struct LoopCtx {
    break_jumps: Vec<usize>,
    continue_jumps: Vec<usize>,
}

impl ProtoBuilder {
    fn new(name: PhpStr) -> Self {
        ProtoBuilder {
            name,
            code: Vec::new(),
            consts: Vec::new(),
            names: Vec::new(),
            cv_map: Vec::new(),
            caches: Vec::new(),
            exception_table: Vec::new(),
            finally_rets: Vec::new(),
            num_cvs: 0,
            next_temp: 0,
            high_water: 0,
            temp_floor: 0,
            loops: Vec::new(),
            sigs: Rc::new(HashMap::new()),
        }
    }

    fn finish(self) -> FunctionProto {
        let mut proto = FunctionProto::new(self.name);
        proto.num_cvs = self.num_cvs;
        proto.num_registers = self.high_water.max(self.num_cvs);
        proto.code = self.code;
        proto.consts = self.consts;
        proto.names = self.names;
        proto.caches = self.caches;
        proto.exception_table = self.exception_table;
        proto.debug.cv_names = (0..self.cv_map.len() as u32).collect();
        proto
    }

    fn alloc_cache(&mut self) -> phrust_bytecode::CacheIdx {
        let idx = self.caches.len() as phrust_bytecode::CacheIdx;
        self.caches.push(phrust_bytecode::InlineCache::default());
        idx
    }

    // -- register / pool management --

    fn emit(&mut self, instr: Instr) {
        self.code.push(instr);
    }

    fn alloc_temp(&mut self) -> Reg {
        let r = self.next_temp;
        self.next_temp += 1;
        self.high_water = self.high_water.max(self.next_temp);
        r
    }

    /// Allocate `n` contiguous temporaries (for a call arg window).
    fn alloc_temps(&mut self, n: u16) -> Reg {
        let base = self.next_temp;
        self.next_temp += n;
        self.high_water = self.high_water.max(self.next_temp);
        base
    }

    /// Free temporaries back to the current floor (end of a statement).
    fn reset_temps(&mut self) {
        self.next_temp = self.temp_floor;
    }

    /// Patch a placeholder `Nop` at `pos` into a jump to `target`.
    fn patch_jmp(&mut self, pos: usize, target: u32) {
        self.code[pos] = Instr::Jmp { target };
    }

    fn here(&self) -> u32 {
        self.code.len() as u32
    }

    fn cv(&mut self, var_name: &[u8]) -> Reg {
        if let Some(i) = self.cv_map.iter().position(|n| n == var_name) {
            return i as Reg;
        }
        let r = self.cv_map.len() as Reg;
        self.cv_map.push(var_name.to_vec());
        r
    }

    fn intern_name(&mut self, bytes: &[u8]) -> NIdx {
        if let Some(i) = self.names.iter().position(|n| n.as_bytes() == bytes) {
            return i as NIdx;
        }
        let i = self.names.len() as NIdx;
        self.names.push(PhpStr::from_bytes(bytes.to_vec()));
        i
    }

    /// Evaluate a parameter default to a constant and intern it. Only
    /// compile-time constants are supported (literals, `[]`, true/false/null,
    /// negated numeric literals); other expressions are deferred.
    fn const_default(&mut self, e: &Expression) -> Result<phrust_bytecode::Default, Thrown> {
        let v = self.const_value(e)?;
        Ok(phrust_bytecode::Default::Const(self.intern_const(v)))
    }

    fn const_value(&mut self, e: &Expression) -> Result<Value, Thrown> {
        let v = match e {
            Expression::Literal(l) => literal_value(l),
            Expression::Parenthesized(p) => return self.const_value(p.expression),
            Expression::Array(a) if a.elements.is_empty() => Value::Array(PhpArray::new()),
            Expression::ConstantAccess(ca) => match identifier_name(&ca.name) {
                b"true" | b"TRUE" | b"True" => Value::Bool(true),
                b"false" | b"FALSE" | b"False" => Value::Bool(false),
                b"null" | b"NULL" | b"Null" => Value::Null,
                _ => {
                    return Err(Thrown::unsupported(
                        "non-constant default value",
                        "only literals / [] / true/false/null are supported for now",
                    ))
                }
            },
            Expression::UnaryPrefix(u) => {
                use UnaryPrefixOperator as Op;
                let inner = self.const_value(u.operand)?;
                match (&u.operator, inner) {
                    (Op::Negation(_), Value::Int(n)) => Value::Int(-n),
                    (Op::Negation(_), Value::Float(f)) => Value::Float(-f),
                    (Op::Plus(_), v @ (Value::Int(_) | Value::Float(_))) => v,
                    _ => {
                        return Err(Thrown::unsupported(
                            "non-constant default value",
                            "only literals / [] / true/false/null are supported for now",
                        ))
                    }
                }
            }
            _ => {
                return Err(Thrown::unsupported(
                    "non-constant default value",
                    "only literals / [] / true/false/null are supported for now",
                ))
            }
        };
        Ok(v)
    }

    fn intern_const(&mut self, v: Value) -> CIdx {
        if let Some(i) = self.consts.iter().position(|c| const_eq(c, &v)) {
            return i as CIdx;
        }
        let i = self.consts.len() as CIdx;
        self.consts.push(v);
        i
    }

    // -- CV pre-pass --

    fn scan_stmt_vars(&mut self, stmt: &Statement) {
        use phrust_frontend::cst::{ForBody, IfBody, SwitchBody, SwitchCase, WhileBody};
        match stmt {
            Statement::Echo(e) => e.values.iter().for_each(|v| self.scan_expr_vars(v)),
            Statement::EchoTag(e) => e.values.iter().for_each(|v| self.scan_expr_vars(v)),
            Statement::Expression(s) => self.scan_expr_vars(s.expression),
            Statement::Block(b) => b.statements.iter().for_each(|s| self.scan_stmt_vars(s)),
            Statement::If(i) => {
                self.scan_expr_vars(i.condition);
                match &i.body {
                    IfBody::Statement(b) => {
                        self.scan_stmt_vars(b.statement);
                        for c in b.else_if_clauses.iter() {
                            self.scan_expr_vars(c.condition);
                            self.scan_stmt_vars(c.statement);
                        }
                        if let Some(e) = &b.else_clause {
                            self.scan_stmt_vars(e.statement);
                        }
                    }
                    IfBody::ColonDelimited(b) => {
                        b.statements.iter().for_each(|s| self.scan_stmt_vars(s));
                        for c in b.else_if_clauses.iter() {
                            self.scan_expr_vars(c.condition);
                            c.statements.iter().for_each(|s| self.scan_stmt_vars(s));
                        }
                        if let Some(e) = &b.else_clause {
                            e.statements.iter().for_each(|s| self.scan_stmt_vars(s));
                        }
                    }
                }
            }
            Statement::While(w) => {
                self.scan_expr_vars(w.condition);
                match &w.body {
                    WhileBody::Statement(s) => self.scan_stmt_vars(s),
                    WhileBody::ColonDelimited(b) => {
                        b.statements.iter().for_each(|s| self.scan_stmt_vars(s))
                    }
                }
            }
            Statement::DoWhile(d) => {
                self.scan_stmt_vars(d.statement);
                self.scan_expr_vars(d.condition);
            }
            Statement::For(f) => {
                f.initializations.iter().for_each(|e| self.scan_expr_vars(e));
                f.conditions.iter().for_each(|e| self.scan_expr_vars(e));
                f.increments.iter().for_each(|e| self.scan_expr_vars(e));
                match &f.body {
                    ForBody::Statement(s) => self.scan_stmt_vars(s),
                    ForBody::ColonDelimited(b) => {
                        b.statements.iter().for_each(|s| self.scan_stmt_vars(s))
                    }
                }
            }
            Statement::Switch(sw) => {
                self.scan_expr_vars(sw.expression);
                let cases = match &sw.body {
                    SwitchBody::BraceDelimited(b) => &b.cases,
                    SwitchBody::ColonDelimited(b) => &b.cases,
                };
                for case in cases.iter() {
                    match case {
                        SwitchCase::Expression(c) => {
                            self.scan_expr_vars(c.expression);
                            c.statements.iter().for_each(|s| self.scan_stmt_vars(s));
                        }
                        SwitchCase::Default(c) => {
                            c.statements.iter().for_each(|s| self.scan_stmt_vars(s))
                        }
                    }
                }
            }
            Statement::Return(r) => {
                if let Some(v) = r.value {
                    self.scan_expr_vars(v);
                }
            }
            Statement::Foreach(fe) => {
                self.scan_expr_vars(fe.expression);
                match &fe.target {
                    ForeachTarget::Value(t) => self.scan_expr_vars(t.value),
                    ForeachTarget::KeyValue(t) => {
                        self.scan_expr_vars(t.key);
                        self.scan_expr_vars(t.value);
                    }
                }
                match &fe.body {
                    ForeachBody::Statement(s) => self.scan_stmt_vars(s),
                    ForeachBody::ColonDelimited(b) => {
                        b.statements.iter().for_each(|s| self.scan_stmt_vars(s))
                    }
                }
            }
            Statement::Unset(u) => u.values.iter().for_each(|v| self.scan_expr_vars(v)),
            Statement::Try(t) => {
                t.block.statements.iter().for_each(|s| self.scan_stmt_vars(s));
                for clause in t.catch_clauses.iter() {
                    if let Some(v) = &clause.variable {
                        self.cv(strip_dollar(v.name));
                    }
                    clause.block.statements.iter().for_each(|s| self.scan_stmt_vars(s));
                }
                if let Some(fin) = &t.finally_clause {
                    fin.block.statements.iter().for_each(|s| self.scan_stmt_vars(s));
                }
            }
            _ => {}
        }
    }

    fn scan_expr_vars(&mut self, expr: &Expression) {
        match expr {
            Expression::Variable(Variable::Direct(d)) => {
                self.cv(strip_dollar(d.name));
            }
            Expression::Parenthesized(p) => self.scan_expr_vars(p.expression),
            Expression::Binary(b) => {
                self.scan_expr_vars(b.lhs);
                self.scan_expr_vars(b.rhs);
            }
            Expression::UnaryPrefix(u) => self.scan_expr_vars(u.operand),
            Expression::UnaryPostfix(u) => self.scan_expr_vars(u.operand),
            Expression::Assignment(a) => {
                self.scan_expr_vars(a.lhs);
                self.scan_expr_vars(a.rhs);
            }
            Expression::Conditional(c) => {
                self.scan_expr_vars(c.condition);
                if let Some(t) = &c.then {
                    self.scan_expr_vars(t);
                }
                self.scan_expr_vars(c.r#else);
            }
            Expression::Array(arr) => self.scan_array_vars(arr.elements.iter()),
            Expression::LegacyArray(arr) => self.scan_array_vars(arr.elements.iter()),
            Expression::List(lst) => self.scan_array_vars(lst.elements.iter()),
            Expression::ArrayAccess(aa) => {
                self.scan_expr_vars(aa.array);
                self.scan_expr_vars(aa.index);
            }
            Expression::ArrayAppend(ap) => self.scan_expr_vars(ap.array),
            Expression::CompositeString(cs) => {
                let parts = match cs {
                    CompositeString::Interpolated(i) => Some(&i.parts),
                    CompositeString::Document(d) => Some(&d.parts),
                    CompositeString::ShellExecute(_) => None,
                };
                if let Some(parts) = parts {
                    for part in parts.iter() {
                        match part {
                            StringPart::Expression(e) => self.scan_expr_vars(e),
                            StringPart::BracedExpression(b) => self.scan_expr_vars(b.expression),
                            StringPart::Literal(_) => {}
                        }
                    }
                }
            }
            Expression::Match(m) => {
                self.scan_expr_vars(m.expression);
                for arm in m.arms.iter() {
                    match arm {
                        phrust_frontend::cst::MatchArm::Expression(a) => {
                            a.conditions.iter().for_each(|c| self.scan_expr_vars(c));
                            self.scan_expr_vars(a.expression);
                        }
                        phrust_frontend::cst::MatchArm::Default(a) => self.scan_expr_vars(a.expression),
                    }
                }
            }
            Expression::Instantiation(inst) => {
                self.scan_expr_vars(inst.class);
                if let Some(list) = &inst.argument_list {
                    self.scan_args(list);
                }
            }
            Expression::Access(acc) => match acc {
                Access::Property(pa) => self.scan_expr_vars(pa.object),
                Access::NullSafeProperty(pa) => self.scan_expr_vars(pa.object),
                // Note: in `C::$x` the `$x` is the static-property NAME, not a local.
                Access::StaticProperty(spa) => self.scan_expr_vars(spa.class),
                Access::ClassConstant(cca) => self.scan_expr_vars(cca.class),
            },
            Expression::Clone(cl) => self.scan_expr_vars(cl.object),
            Expression::Throw(t) => self.scan_expr_vars(t.exception),
            Expression::Construct(c) => self.scan_construct_vars(c),
            Expression::Call(call) => {
                use phrust_frontend::cst::Call;
                match call {
                    Call::Function(fc) => self.scan_args(&fc.argument_list),
                    Call::Method(mc) => {
                        self.scan_expr_vars(mc.object);
                        self.scan_args(&mc.argument_list);
                    }
                    Call::NullSafeMethod(mc) => {
                        self.scan_expr_vars(mc.object);
                        self.scan_args(&mc.argument_list);
                    }
                    Call::StaticMethod(smc) => {
                        self.scan_expr_vars(smc.class);
                        self.scan_args(&smc.argument_list);
                    }
                }
            }
            _ => {}
        }
    }

    fn scan_args(&mut self, list: &phrust_frontend::cst::ArgumentList) {
        for arg in list.arguments.iter() {
            match arg {
                Argument::Positional(p) => self.scan_expr_vars(p.value),
                Argument::Named(n) => self.scan_expr_vars(n.value),
            }
        }
    }

    fn scan_array_vars<'a>(&mut self, els: impl Iterator<Item = &'a ArrayElement<'a>>) {
        for el in els {
            match el {
                ArrayElement::KeyValue(kv) => {
                    self.scan_expr_vars(kv.key);
                    self.scan_expr_vars(kv.value);
                }
                ArrayElement::Value(v) => self.scan_expr_vars(v.value),
                ArrayElement::Variadic(v) => self.scan_expr_vars(v.value),
                ArrayElement::Missing(_) => {}
            }
        }
    }

    fn scan_construct_vars(&mut self, c: &Construct) {
        match c {
            Construct::Print(p) => self.scan_expr_vars(p.value),
            Construct::Empty(e) => self.scan_expr_vars(e.value),
            Construct::Isset(i) => i.values.iter().for_each(|v| self.scan_expr_vars(v)),
            _ => {}
        }
    }

    // -- statement lowering --

    fn compile_stmt(&mut self, stmt: &Statement) -> Result<(), Thrown> {
        match stmt {
            Statement::OpeningTag(_) | Statement::ClosingTag(_) | Statement::Noop(_) => {}
            Statement::Inline(inline) => {
                if inline.kind.is_text() {
                    let k = self.intern_const(Value::str(inline.value.to_vec()));
                    self.emit(Instr::EchoConst { k });
                }
            }
            Statement::Echo(e) => {
                for v in e.values.iter() {
                    let r = self.compile_expr(v)?;
                    self.emit(Instr::Echo { src: r });
                }
            }
            Statement::EchoTag(e) => {
                for v in e.values.iter() {
                    let r = self.compile_expr(v)?;
                    self.emit(Instr::Echo { src: r });
                }
            }
            Statement::Expression(s) => {
                self.compile_expr(s.expression)?;
            }
            Statement::Block(b) => {
                for s in b.statements.iter() {
                    self.compile_stmt(s)?;
                }
            }
            Statement::HaltCompiler(_) => self.emit(Instr::Halt),
            // Top-level function + class/interface declarations are hoisted.
            Statement::Function(_) => {}
            Statement::Class(_) => {}
            Statement::Interface(_) => {}
            Statement::Trait(_) => {}
            Statement::Enum(_) => {}
            Statement::If(i) => self.compile_if(i)?,
            Statement::While(w) => self.compile_while(w)?,
            Statement::DoWhile(d) => self.compile_do_while(d)?,
            Statement::For(f) => self.compile_for(f)?,
            Statement::Switch(s) => self.compile_switch(s)?,
            Statement::Foreach(fe) => self.compile_foreach(fe)?,
            Statement::Try(t) => self.compile_try(t)?,
            Statement::Unset(u) => self.compile_unset(u)?,
            Statement::Return(r) => {
                if self.finally_rets.is_empty() {
                    match r.value {
                        Some(e) => {
                            let src = self.compile_expr(e)?;
                            self.emit(Instr::Ret { src });
                        }
                        None => self.emit(Instr::RetNull),
                    }
                } else {
                    // Route the return through the innermost enclosing finally.
                    let src = match r.value {
                        Some(e) => self.compile_expr(e)?,
                        None => {
                            let t = self.alloc_temp();
                            self.emit(Instr::LoadNull { dst: t });
                            t
                        }
                    };
                    self.emit(Instr::SetPendingRet { src });
                    let pos = self.code.len();
                    self.emit(Instr::Nop); // Jmp -> finally (patched)
                    self.finally_rets.last_mut().unwrap().push(pos);
                }
            }
            Statement::Break(b) => self.compile_break_continue(b.level, true)?,
            Statement::Continue(c) => self.compile_break_continue(c.level, false)?,
            other => {
                return Err(Thrown::unsupported(
                    format!("{} statement", stmt_kind(other)),
                    "not implemented in the phase-1 compiler",
                ))
            }
        }
        self.reset_temps();
        Ok(())
    }

    // -- control-flow lowering --

    /// Compile a single statement or a colon-delimited statement sequence.
    fn compile_stmts<'a>(&mut self, stmts: impl Iterator<Item = &'a Statement<'a>>) -> Result<(), Thrown> {
        for s in stmts {
            self.compile_stmt(s)?;
        }
        Ok(())
    }

    fn compile_if(&mut self, i: &phrust_frontend::cst::If) -> Result<(), Thrown> {
        // Jumps to the end of the whole if/elseif/else chain, patched at the end.
        let mut end_jumps = Vec::new();

        // Emit one (condition, body) clause: returns nothing; leaves a JmpFalse
        // targeting the next clause's start.
        macro_rules! clause {
            ($cond:expr, $body:block) => {{
                let cond = self.compile_expr($cond)?;
                let jf = self.code.len();
                self.emit(Instr::Nop); // JmpFalse cond -> next clause
                $body
                end_jumps.push(self.code.len());
                self.emit(Instr::Nop); // Jmp -> end
                let next = self.here();
                self.code[jf] = Instr::JmpFalse { cond, target: next };
            }};
        }

        match &i.body {
            IfBody::Statement(b) => {
                clause!(i.condition, {
                    self.compile_stmt(b.statement)?;
                });
                for c in b.else_if_clauses.iter() {
                    clause!(c.condition, {
                        self.compile_stmt(c.statement)?;
                    });
                }
                if let Some(e) = &b.else_clause {
                    self.compile_stmt(e.statement)?;
                }
            }
            IfBody::ColonDelimited(b) => {
                clause!(i.condition, {
                    self.compile_stmts(b.statements.iter())?;
                });
                for c in b.else_if_clauses.iter() {
                    clause!(c.condition, {
                        self.compile_stmts(c.statements.iter())?;
                    });
                }
                if let Some(e) = &b.else_clause {
                    self.compile_stmts(e.statements.iter())?;
                }
            }
        }

        let end = self.here();
        for j in end_jumps {
            self.patch_jmp(j, end);
        }
        Ok(())
    }

    fn compile_while(&mut self, w: &phrust_frontend::cst::While) -> Result<(), Thrown> {
        let top = self.here();
        let cond = self.compile_expr(w.condition)?;
        let jf = self.code.len();
        self.emit(Instr::Nop); // JmpFalse cond -> end
        self.loops.push(LoopCtx::default());
        match &w.body {
            WhileBody::Statement(s) => self.compile_stmt(s)?,
            WhileBody::ColonDelimited(b) => self.compile_stmts(b.statements.iter())?,
        }
        let ctx = self.loops.pop().unwrap();
        self.emit(Instr::Jmp { target: top });
        let end = self.here();
        self.code[jf] = Instr::JmpFalse { cond, target: end };
        self.patch_all(ctx.break_jumps, end);
        self.patch_all(ctx.continue_jumps, top);
        Ok(())
    }

    fn compile_do_while(&mut self, d: &phrust_frontend::cst::DoWhile) -> Result<(), Thrown> {
        let top = self.here();
        self.loops.push(LoopCtx::default());
        self.compile_stmt(d.statement)?;
        let ctx = self.loops.pop().unwrap();
        let cont = self.here(); // continue -> re-test the condition
        let cond = self.compile_expr(d.condition)?;
        self.emit(Instr::JmpTrue { cond, target: top });
        let end = self.here();
        self.patch_all(ctx.break_jumps, end);
        self.patch_all(ctx.continue_jumps, cont);
        Ok(())
    }

    fn compile_for(&mut self, f: &phrust_frontend::cst::For) -> Result<(), Thrown> {
        for e in f.initializations.iter() {
            self.compile_expr(e)?;
        }
        let top = self.here();
        // All conditions evaluated for side effects; the LAST controls the loop.
        let jf = if f.conditions.is_empty() {
            None
        } else {
            let mut last = None;
            for e in f.conditions.iter() {
                last = Some(self.compile_expr(e)?);
            }
            let pos = self.code.len();
            self.emit(Instr::Nop); // JmpFalse last -> end
            Some((last.unwrap(), pos))
        };
        self.loops.push(LoopCtx::default());
        match &f.body {
            ForBody::Statement(s) => self.compile_stmt(s)?,
            ForBody::ColonDelimited(b) => self.compile_stmts(b.statements.iter())?,
        }
        let ctx = self.loops.pop().unwrap();
        let cont = self.here(); // continue -> increments
        for e in f.increments.iter() {
            self.compile_expr(e)?;
        }
        self.emit(Instr::Jmp { target: top });
        let end = self.here();
        if let Some((cond, pos)) = jf {
            self.code[pos] = Instr::JmpFalse { cond, target: end };
        }
        self.patch_all(ctx.break_jumps, end);
        self.patch_all(ctx.continue_jumps, cont);
        Ok(())
    }

    fn compile_switch(&mut self, sw: &phrust_frontend::cst::Switch) -> Result<(), Thrown> {
        let subject = self.compile_expr(sw.expression)?;
        // Protect the subject register across the fall-through case bodies.
        let saved_floor = self.temp_floor;
        self.temp_floor = self.next_temp;

        let cases: Vec<&SwitchCase> = match &sw.body {
            SwitchBody::BraceDelimited(b) => b.cases.iter().collect(),
            SwitchBody::ColonDelimited(b) => b.cases.iter().collect(),
        };

        // Dispatch: compare the subject against each case (loose ==), in order.
        let mut dispatch: Vec<(usize, usize, Reg)> = Vec::new(); // (case idx, jmp pos, cond reg)
        for (idx, case) in cases.iter().enumerate() {
            if let SwitchCase::Expression(c) = case {
                let cexpr = self.compile_expr(c.expression)?;
                let t = self.alloc_temp();
                self.emit(Instr::Cmp { dst: t, a: subject, b: cexpr, kind: CmpKind::Eq });
                let pos = self.code.len();
                self.emit(Instr::Nop); // JmpTrue t -> case body
                dispatch.push((idx, pos, t));
            }
        }
        let default_idx = cases.iter().position(|c| matches!(c, SwitchCase::Default(_)));
        let no_match_pos = self.code.len();
        self.emit(Instr::Nop); // Jmp -> default body or end

        // Case bodies in source order (fall-through: no jumps between them).
        self.loops.push(LoopCtx::default()); // switch is one break/continue level
        let mut starts = vec![0u32; cases.len()];
        for (idx, case) in cases.iter().enumerate() {
            starts[idx] = self.here();
            match case {
                SwitchCase::Expression(c) => self.compile_stmts(c.statements.iter())?,
                SwitchCase::Default(c) => self.compile_stmts(c.statements.iter())?,
            }
        }
        let ctx = self.loops.pop().unwrap();
        let end = self.here();

        for (idx, pos, cond) in dispatch {
            self.code[pos] = Instr::JmpTrue { cond, target: starts[idx] };
        }
        let no_match_target = default_idx.map(|i| starts[i]).unwrap_or(end);
        self.patch_jmp(no_match_pos, no_match_target);
        // In a switch, continue (level 1) behaves like break.
        self.patch_all(ctx.break_jumps, end);
        self.patch_all(ctx.continue_jumps, end);

        self.temp_floor = saved_floor;
        Ok(())
    }

    fn compile_foreach(&mut self, fe: &phrust_frontend::cst::Foreach) -> Result<(), Thrown> {
        let src = self.compile_expr(fe.expression)?;
        let arr_reg = self.alloc_temp();
        let pos_reg = self.alloc_temp();

        // Resolve key + value targets. A value target may be a destructuring
        // pattern, which binds into a protected element register per iteration.
        let (key_cv, val_expr) = match &fe.target {
            ForeachTarget::Value(t) => (Reg::MAX, t.value),
            ForeachTarget::KeyValue(t) => (self.foreach_target_cv(t.key)?, t.value),
        };
        let (val_cv, val_pattern): (Reg, Option<&Expression>) = match val_expr {
            Expression::Variable(Variable::Direct(d)) => (self.cv(strip_dollar(d.name)), None),
            Expression::UnaryPrefix(u)
                if matches!(u.operator, UnaryPrefixOperator::Reference(_)) =>
            {
                return Err(Thrown::unsupported(
                    "foreach by reference",
                    "references arrive in the next phase",
                ))
            }
            Expression::Array(_) | Expression::List(_) => (self.alloc_temp(), Some(val_expr)),
            _ => return Err(Thrown::unsupported("foreach target", "variable or destructuring only")),
        };

        let saved_floor = self.temp_floor;
        self.temp_floor = self.next_temp; // protect iterator + element regs across the body

        let reset_pos = self.code.len();
        self.emit(Instr::Nop); // FeReset (needs end)
        let fetch_label = self.here();
        let fetch_pos = self.code.len();
        self.emit(Instr::Nop); // FeFetch (needs end)

        // Destructure the fetched element into the pattern, if any.
        if let Some(pattern) = val_pattern {
            match pattern {
                Expression::Array(arr) => self.destructure_into(arr.elements.iter(), val_cv)?,
                Expression::List(lst) => self.destructure_into(lst.elements.iter(), val_cv)?,
                _ => unreachable!(),
            }
        }

        self.loops.push(LoopCtx::default());
        match &fe.body {
            ForeachBody::Statement(s) => self.compile_stmt(s)?,
            ForeachBody::ColonDelimited(b) => self.compile_stmts(b.statements.iter())?,
        }
        let ctx = self.loops.pop().unwrap();
        self.emit(Instr::Jmp { target: fetch_label });
        let end = self.here();
        self.code[reset_pos] = Instr::FeReset { arr_reg, pos_reg, src, end };
        self.code[fetch_pos] = Instr::FeFetch { arr_reg, pos_reg, key_cv, val_cv, end };
        self.patch_all(ctx.break_jumps, end);
        self.patch_all(ctx.continue_jumps, fetch_label);
        self.temp_floor = saved_floor;
        Ok(())
    }

    fn foreach_target_cv(&mut self, e: &Expression) -> Result<Reg, Thrown> {
        match e {
            Expression::Variable(Variable::Direct(d)) => Ok(self.cv(strip_dollar(d.name))),
            Expression::UnaryPrefix(u)
                if matches!(u.operator, UnaryPrefixOperator::Reference(_)) =>
            {
                Err(Thrown::unsupported(
                    "foreach by reference",
                    "references arrive in the next phase",
                ))
            }
            _ => Err(Thrown::unsupported(
                "foreach target",
                "only simple `$var` targets for now (no list()/destructuring)",
            )),
        }
    }

    fn compile_unset(&mut self, u: &phrust_frontend::cst::Unset) -> Result<(), Thrown> {
        for v in u.values.iter() {
            match v {
                Expression::Variable(Variable::Direct(d)) => {
                    // A CV can't be removed; null is observably equivalent to
                    // unset for isset/??/empty.
                    let cv = self.cv(strip_dollar(d.name));
                    self.emit(Instr::LoadNull { dst: cv });
                }
                Expression::ArrayAccess(_) => {
                    let mut steps = Vec::new();
                    let root = self.collect_lvalue_steps(v, &mut steps)?;
                    self.emit_cursor(root, &steps);
                    self.emit(Instr::UnsetLval);
                }
                // `unset($o->prop)`: remove a declared property, else `__unset`.
                Expression::Access(Access::Property(pa)) => {
                    if let ClassLikeMemberSelector::Identifier(li) = &pa.property {
                        let obj = self.compile_expr(pa.object)?;
                        let name = self.intern_name(li.value);
                        self.emit(Instr::UnsetProp { obj, name });
                    } else {
                        return Err(Thrown::unsupported(
                            "unset of dynamic property name",
                            "`unset($o->$x)` deferred",
                        ));
                    }
                }
                _ => {
                    return Err(Thrown::unsupported(
                        "unset target",
                        "only variables, array elements, and properties",
                    ))
                }
            }
        }
        Ok(())
    }

    fn compile_try(&mut self, t: &phrust_frontend::cst::Try) -> Result<(), Thrown> {
        use phrust_bytecode::{CatchArm, ExRegion};
        let has_finally = t.finally_clause.is_some();
        if has_finally {
            self.finally_rets.push(Vec::new());
        }
        let try_start = self.here();
        self.compile_stmts(t.block.statements.iter())?;
        let try_end = self.here();
        // Normal completion jumps past the catch bodies to finally/after.
        let norm_jmp = self.code.len();
        self.emit(Instr::Nop);

        let mut catch_arms = Vec::new();
        let mut end_jumps = vec![norm_jmp];
        for clause in t.catch_clauses.iter() {
            let target = self.here();
            let classes = hint_class_names(&clause.hint)?;
            let var_reg = clause.variable.as_ref().map(|v| self.cv(strip_dollar(v.name)));
            self.compile_stmts(clause.block.statements.iter())?;
            let j = self.code.len();
            self.emit(Instr::Nop);
            end_jumps.push(j);
            catch_arms.push(CatchArm {
                classes,
                var_reg,
                target,
            });
        }

        let finally_pc = if let Some(fin) = &t.finally_clause {
            // Stop routing returns into THIS finally while compiling its body.
            let pending_rets = self.finally_rets.pop().unwrap();
            let fpc = self.here();
            self.compile_stmts(fin.block.statements.iter())?;
            self.emit(Instr::FinallyEnd);
            // Patch `return`s that crossed out of try/catch to enter the finally.
            for pos in pending_rets {
                self.code[pos] = Instr::Jmp { target: fpc };
            }
            Some(fpc)
        } else {
            None
        };

        let after = self.here();
        let norm_target = finally_pc.unwrap_or(after);
        for j in end_jumps {
            self.patch_jmp(j, norm_target);
        }

        self.exception_table.push(ExRegion {
            start: try_start,
            end: try_end,
            catches: catch_arms,
            finally: finally_pc,
        });
        Ok(())
    }

    /// Assign the elements of `src` (an array value) to a destructuring pattern.
    fn destructure_into<'a>(
        &mut self,
        elements: impl Iterator<Item = &'a ArrayElement<'a>>,
        src: Reg,
    ) -> Result<(), Thrown> {
        let mut idx: i64 = 0;
        for el in elements {
            match el {
                ArrayElement::Missing(_) => idx += 1,
                ArrayElement::Value(v) => {
                    let key = self.alloc_temp();
                    self.emit(Instr::LoadInt { dst: key, n: idx });
                    let val = self.alloc_temp();
                    self.emit(Instr::DimFetch { dst: val, arr: src, key });
                    self.store_to_target(v.value, val)?;
                    idx += 1;
                }
                ArrayElement::KeyValue(kv) => {
                    let key = self.compile_expr(kv.key)?;
                    let val = self.alloc_temp();
                    self.emit(Instr::DimFetch { dst: val, arr: src, key });
                    self.store_to_target(kv.value, val)?;
                }
                ArrayElement::Variadic(_) => {
                    return Err(Thrown::unsupported("spread in destructuring", "deferred"))
                }
            }
        }
        Ok(())
    }

    /// Store `src` into a single destructuring target (variable, nested pattern,
    /// array element, or property).
    fn store_to_target(&mut self, target: &Expression, src: Reg) -> Result<(), Thrown> {
        match target {
            Expression::Variable(Variable::Direct(d)) => {
                let cv = self.cv(strip_dollar(d.name));
                self.emit(Instr::Assign { cv, src });
            }
            Expression::Array(arr) => self.destructure_into(arr.elements.iter(), src)?,
            Expression::List(lst) => self.destructure_into(lst.elements.iter(), src)?,
            Expression::ArrayAccess(_)
            | Expression::ArrayAppend(_)
            | Expression::Access(Access::Property(_))
            | Expression::Access(Access::StaticProperty(_)) => {
                let mut steps = Vec::new();
                let root = self.collect_lvalue_steps(target, &mut steps)?;
                self.emit_cursor(root, &steps);
                self.emit(Instr::AssignLval { src });
            }
            _ => return Err(Thrown::unsupported("destructuring target", "deferred")),
        }
        Ok(())
    }

    fn compile_break_continue(
        &mut self,
        level: Option<&Expression>,
        is_break: bool,
    ) -> Result<(), Thrown> {
        let n = match level {
            None => 1usize,
            Some(Expression::Literal(Literal::Integer(i))) => i.value.unwrap_or(1).max(1) as usize,
            Some(_) => {
                return Err(Thrown::error(
                    "Error",
                    "'break'/'continue' operator accepts only a constant integer level",
                ))
            }
        };
        let kw = if is_break { "break" } else { "continue" };
        if n == 0 || self.loops.len() < n {
            return Err(Thrown::error(
                "Error",
                format!("Cannot '{kw}' {n} level(s)"),
            ));
        }
        let idx = self.loops.len() - n;
        let pos = self.code.len();
        self.emit(Instr::Nop); // patched to Jmp at loop close
        if is_break {
            self.loops[idx].break_jumps.push(pos);
        } else {
            self.loops[idx].continue_jumps.push(pos);
        }
        Ok(())
    }

    fn patch_all(&mut self, positions: Vec<usize>, target: u32) {
        for p in positions {
            self.patch_jmp(p, target);
        }
    }

    fn compile_match(&mut self, m: &Match) -> Result<Reg, Thrown> {
        let subject = self.compile_expr(m.expression)?;
        let dst = self.alloc_temp();
        let arms: Vec<&MatchArm> = m.arms.iter().collect();

        // Dispatch: strict === against each arm condition, in order.
        let mut dispatch: Vec<(usize, usize, Reg)> = Vec::new();
        let mut default_idx = None;
        for (idx, arm) in arms.iter().enumerate() {
            match arm {
                MatchArm::Expression(a) => {
                    for cond in a.conditions.iter() {
                        let c = self.compile_expr(cond)?;
                        let t = self.alloc_temp();
                        self.emit(Instr::Cmp { dst: t, a: subject, b: c, kind: CmpKind::Identical });
                        let pos = self.code.len();
                        self.emit(Instr::Nop); // JmpTrue t -> arm body
                        dispatch.push((idx, pos, t));
                    }
                }
                MatchArm::Default(_) => default_idx = Some(idx),
            }
        }
        let no_match_pos = self.code.len();
        self.emit(Instr::Nop); // Jmp -> default arm or throw

        // Arm bodies.
        let mut starts = vec![0u32; arms.len()];
        let mut end_jumps = Vec::new();
        for (idx, arm) in arms.iter().enumerate() {
            starts[idx] = self.here();
            let e = match arm {
                MatchArm::Expression(a) => a.expression,
                MatchArm::Default(a) => a.expression,
            };
            let r = self.compile_expr(e)?;
            self.emit(Instr::Mov { dst, src: r });
            end_jumps.push(self.code.len());
            self.emit(Instr::Nop); // Jmp -> end
        }
        let throw_label = self.here();
        self.emit(Instr::ThrowMatchError { subject });
        let end = self.here();

        for (idx, pos, cond) in dispatch {
            self.code[pos] = Instr::JmpTrue { cond, target: starts[idx] };
        }
        let nm_target = default_idx.map(|i| starts[i]).unwrap_or(throw_label);
        self.patch_jmp(no_match_pos, nm_target);
        for j in end_jumps {
            self.patch_jmp(j, end);
        }
        Ok(dst)
    }

    // -- expression lowering (returns the register holding the result) --

    fn compile_expr(&mut self, expr: &Expression) -> Result<Reg, Thrown> {
        match expr {
            Expression::Literal(lit) => self.compile_literal(lit),
            Expression::Parenthesized(p) => self.compile_expr(p.expression),
            Expression::Variable(Variable::Direct(d)) => Ok(self.cv(strip_dollar(d.name))),
            Expression::Variable(_) => Err(Thrown::unsupported(
                "indirect/nested variable",
                "variable-variables are refused / deferred",
            )),
            Expression::Binary(b) => self.compile_binary(b),
            Expression::UnaryPrefix(u) => self.compile_unary_prefix(u),
            Expression::UnaryPostfix(u) => self.compile_unary_postfix(u),
            Expression::Assignment(a) => self.compile_assignment(a),
            Expression::Conditional(c) => self.compile_conditional(c),
            Expression::Array(arr) => self.compile_array(arr.elements.iter()),
            Expression::LegacyArray(arr) => self.compile_array(arr.elements.iter()),
            Expression::ArrayAccess(aa) => {
                let arr = self.compile_expr(aa.array)?;
                let key = self.compile_expr(aa.index)?;
                let dst = self.alloc_temp();
                self.emit(Instr::DimFetch { dst, arr, key });
                Ok(dst)
            }
            Expression::ConstantAccess(ca) => self.compile_constant(&ca.name),
            Expression::Identifier(id) => self.compile_constant(id),
            Expression::CompositeString(cs) => self.compile_composite(cs),
            Expression::Match(m) => self.compile_match(m),
            Expression::Instantiation(inst) => self.compile_new(inst),
            Expression::Access(acc) => self.compile_access(acc),
            Expression::Clone(cl) => {
                let src = self.compile_expr(cl.object)?;
                let dst = self.alloc_temp();
                self.emit(Instr::CloneObj { dst, src });
                Ok(dst)
            }
            Expression::Throw(t) => {
                let src = self.compile_expr(t.exception)?;
                self.emit(Instr::Throw { src });
                // `throw` never returns; the dst is unreachable.
                let dst = self.alloc_temp();
                Ok(dst)
            }
            Expression::Construct(c) => self.compile_construct(c),
            Expression::Call(call) => self.compile_call(call),
            Expression::ArrowFunction(af) => self.compile_arrow(af),
            Expression::Closure(cl) => self.compile_closure(cl),
            other => Err(Thrown::unsupported(
                format!("{} expression", expr_kind(other)),
                "not implemented in the phase-1 compiler",
            )),
        }
    }

    fn compile_literal(&mut self, lit: &Literal) -> Result<Reg, Thrown> {
        let dst = self.alloc_temp();
        match literal_value(lit) {
            Value::Null => self.emit(Instr::LoadNull { dst }),
            Value::Bool(b) => self.emit(Instr::LoadBool { dst, b }),
            Value::Int(n) => self.emit(Instr::LoadInt { dst, n }),
            v => {
                let k = self.intern_const(v);
                self.emit(Instr::LoadConst { dst, k });
            }
        }
        Ok(dst)
    }

    fn compile_binary(&mut self, b: &phrust_frontend::cst::Binary) -> Result<Reg, Thrown> {
        use BinaryOperator as Op;
        // Short-circuit operators lower to branches.
        match &b.operator {
            Op::And(_) | Op::LowAnd(_) => return self.compile_logical(b, true),
            Op::Or(_) | Op::LowOr(_) => return self.compile_logical(b, false),
            Op::NullCoalesce(_) => return self.compile_coalesce(b),
            Op::Instanceof(_) => {
                let val = self.compile_expr(b.lhs)?;
                let class = self.class_ref(b.rhs)?;
                let dst = self.alloc_temp();
                self.emit(Instr::InstanceOf { dst, val, class });
                return Ok(dst);
            }
            _ => {}
        }

        let a = self.compile_expr(b.lhs)?;
        let r = self.compile_expr(b.rhs)?;
        let dst = self.alloc_temp();
        let cmp = |k| Instr::Cmp { dst, a, b: r, kind: k };
        let instr = match &b.operator {
            Op::Addition(_) => Instr::Add { dst, a, b: r },
            Op::Subtraction(_) => Instr::Sub { dst, a, b: r },
            Op::Multiplication(_) => Instr::Mul { dst, a, b: r },
            Op::Division(_) => Instr::Div { dst, a, b: r },
            Op::Modulo(_) => Instr::Mod { dst, a, b: r },
            Op::Exponentiation(_) => Instr::Pow { dst, a, b: r },
            Op::BitwiseAnd(_) => Instr::BitAnd { dst, a, b: r },
            Op::BitwiseOr(_) => Instr::BitOr { dst, a, b: r },
            Op::BitwiseXor(_) => Instr::BitXor { dst, a, b: r },
            Op::LeftShift(_) => Instr::Shl { dst, a, b: r },
            Op::RightShift(_) => Instr::Shr { dst, a, b: r },
            Op::StringConcat(_) => Instr::Concat { dst, a, b: r },
            Op::Equal(_) => cmp(CmpKind::Eq),
            Op::NotEqual(_) | Op::AngledNotEqual(_) => cmp(CmpKind::Ne),
            Op::Identical(_) => cmp(CmpKind::Identical),
            Op::NotIdentical(_) => cmp(CmpKind::NotIdentical),
            Op::LessThan(_) => cmp(CmpKind::Lt),
            Op::LessThanOrEqual(_) => cmp(CmpKind::Le),
            Op::GreaterThan(_) => cmp(CmpKind::Gt),
            Op::GreaterThanOrEqual(_) => cmp(CmpKind::Ge),
            Op::Spaceship(_) => cmp(CmpKind::Spaceship),
            Op::LowXor(_) => {
                // a xor b == (bool)a != (bool)b, via cast+compare
                Instr::Cmp { dst, a, b: r, kind: CmpKind::Ne }
            }
            Op::And(_)
            | Op::Or(_)
            | Op::LowAnd(_)
            | Op::LowOr(_)
            | Op::NullCoalesce(_)
            | Op::Instanceof(_) => {
                unreachable!("handled above")
            }
        };
        self.emit(instr);
        Ok(dst)
    }

    /// `&&`/`and` (is_and=true) or `||`/`or` lowered to short-circuit branches,
    /// materializing a bool in `dst`. On the short path `&&`→false, `||`→true;
    /// on the full path `dst = (bool)rhs`.
    fn compile_logical(
        &mut self,
        b: &phrust_frontend::cst::Binary,
        is_and: bool,
    ) -> Result<Reg, Thrown> {
        let dst = self.alloc_temp();
        let a = self.compile_expr(b.lhs)?;
        let jcond = self.code.len();
        self.emit(Instr::Nop); // JmpFalse(&&)/JmpTrue(||) a -> short
        let rb = self.compile_expr(b.rhs)?;
        self.emit(Instr::CastBool { dst, a: rb });
        let jend = self.code.len();
        self.emit(Instr::Nop); // Jmp end
        let short = self.code.len() as u32;
        self.emit(Instr::LoadBool { dst, b: !is_and });
        let end = self.code.len() as u32;
        self.code[jcond] = if is_and {
            Instr::JmpFalse { cond: a, target: short }
        } else {
            Instr::JmpTrue { cond: a, target: short }
        };
        self.code[jend] = Instr::Jmp { target: end };
        Ok(dst)
    }

    fn compile_coalesce(&mut self, b: &phrust_frontend::cst::Binary) -> Result<Reg, Thrown> {
        let dst = self.alloc_temp();
        let a = self.compile_expr(b.lhs)?;
        self.emit(Instr::Mov { dst, src: a });
        let jnn = self.code.len();
        self.emit(Instr::Nop); // JmpNotNull dst -> end
        let r = self.compile_expr(b.rhs)?;
        self.emit(Instr::Mov { dst, src: r });
        let end = self.code.len() as u32;
        self.code[jnn] = Instr::JmpNotNull { src: dst, target: end };
        Ok(dst)
    }

    fn compile_unary_prefix(
        &mut self,
        u: &phrust_frontend::cst::UnaryPrefix,
    ) -> Result<Reg, Thrown> {
        use UnaryPrefixOperator as Op;
        match &u.operator {
            Op::PreIncrement(_) => return self.compile_inc_dec(u.operand, true, 1),
            Op::PreDecrement(_) => return self.compile_inc_dec(u.operand, true, -1),
            Op::UnsetCast(..) | Op::VoidCast(..) | Op::RealCast(..) => {
                return Err(Thrown::unsupported(
                    "(unset)/(void)/(real) cast",
                    "removed/refused in PHP 8",
                ))
            }
            Op::Reference(_) => {
                return Err(Thrown::unsupported("& reference", "references arrive in phase 6"))
            }
            _ => {}
        }
        let a = self.compile_expr(u.operand)?;
        let dst = self.alloc_temp();
        let instr = match &u.operator {
            Op::Negation(_) => Instr::Neg { dst, a },
            Op::Plus(_) => Instr::CastInt { dst, a }, // +x coerces to number; refined below
            Op::Not(_) => Instr::Not { dst, a },
            Op::BitwiseNot(_) => Instr::BitNot { dst, a },
            Op::ErrorControl(_) => Instr::Mov { dst, src: a },
            Op::IntCast(..) | Op::IntegerCast(..) => Instr::CastInt { dst, a },
            Op::FloatCast(..) | Op::DoubleCast(..) => Instr::CastFloat { dst, a },
            Op::StringCast(..) | Op::BinaryCast(..) => Instr::CastString { dst, a },
            Op::BoolCast(..) | Op::BooleanCast(..) => Instr::CastBool { dst, a },
            Op::ArrayCast(..) => Instr::CastArray { dst, a },
            Op::ObjectCast(..) => {
                return Err(Thrown::unsupported("(object) cast", "objects arrive in phase 7"))
            }
            _ => unreachable!("handled above"),
        };
        // `+x` is "unary plus" = numeric coercion, not (int). Use Neg-of-Neg? No:
        // emit Mul by 1 would change int->int; simplest faithful lowering is a
        // dedicated path — represent +x as (x * 1) via Add with 0.
        if matches!(u.operator, Op::Plus(_)) {
            // +x == 0 + x  → preserves int/float per ops::add numeric coercion
            let zero = self.alloc_temp();
            self.emit(Instr::LoadInt { dst: zero, n: 0 });
            self.emit(Instr::Add { dst, a: zero, b: a });
            return Ok(dst);
        }
        self.emit(instr);
        Ok(dst)
    }

    fn compile_unary_postfix(
        &mut self,
        u: &phrust_frontend::cst::UnaryPostfix,
    ) -> Result<Reg, Thrown> {
        use phrust_frontend::cst::UnaryPostfixOperator as Op;
        match &u.operator {
            Op::PostIncrement(_) => self.compile_inc_dec(u.operand, false, 1),
            Op::PostDecrement(_) => self.compile_inc_dec(u.operand, false, -1),
        }
    }

    fn compile_inc_dec(
        &mut self,
        target: &Expression,
        pre: bool,
        delta: i8,
    ) -> Result<Reg, Thrown> {
        match target {
            Expression::Variable(Variable::Direct(d)) => {
                let cv = self.cv(strip_dollar(d.name));
                let dst = self.alloc_temp();
                self.emit(Instr::IncDec { dst, cv, pre, delta });
                Ok(dst)
            }
            Expression::ArrayAccess(_)
            | Expression::Access(Access::Property(_))
            | Expression::Access(Access::StaticProperty(_)) => {
                let mut steps = Vec::new();
                let root = self.collect_lvalue_steps(target, &mut steps)?;
                self.emit_cursor(root, &steps);
                let dst = self.alloc_temp();
                self.emit(Instr::IncDecLval { dst, pre, delta });
                Ok(dst)
            }
            _ => Err(Thrown::unsupported("++/-- target", "variables, array elements, properties")),
        }
    }

    fn compile_conditional(
        &mut self,
        c: &phrust_frontend::cst::Conditional,
    ) -> Result<Reg, Thrown> {
        let dst = self.alloc_temp();
        let cond = self.compile_expr(c.condition)?;
        let jf = self.code.len();
        self.emit(Instr::Nop); // JmpFalse cond -> else
        match &c.then {
            Some(then) => {
                let t = self.compile_expr(then)?;
                self.emit(Instr::Mov { dst, src: t });
            }
            None => {
                // Elvis: reuse the condition value when truthy.
                self.emit(Instr::Mov { dst, src: cond });
            }
        }
        let jend = self.code.len();
        self.emit(Instr::Nop); // Jmp end
        let else_label = self.code.len() as u32;
        let e = self.compile_expr(c.r#else)?;
        self.emit(Instr::Mov { dst, src: e });
        let end = self.code.len() as u32;
        self.code[jf] = Instr::JmpFalse { cond, target: else_label };
        self.code[jend] = Instr::Jmp { target: end };
        Ok(dst)
    }

    fn compile_array<'a>(
        &mut self,
        els: impl Iterator<Item = &'a ArrayElement<'a>>,
    ) -> Result<Reg, Thrown> {
        let dst = self.alloc_temp();
        self.emit(Instr::NewArray { dst, cap: 0 });
        for el in els {
            match el {
                ArrayElement::KeyValue(kv) => {
                    let key = self.compile_expr(kv.key)?;
                    let val = self.compile_expr(kv.value)?;
                    self.emit(Instr::ArrSet { arr: dst, key, val });
                }
                ArrayElement::Value(v) => {
                    let val = self.compile_expr(v.value)?;
                    self.emit(Instr::ArrPush { arr: dst, val });
                }
                ArrayElement::Variadic(v) => {
                    let src = self.compile_expr(v.value)?;
                    self.emit(Instr::ArrSpread { arr: dst, src });
                }
                ArrayElement::Missing(_) => {}
            }
        }
        Ok(dst)
    }

    /// Interpolated (`"...$x..."`) and heredoc strings: concatenate the parts,
    /// stringifying each via `Concat` (which routes to `ops::concat` →
    /// `conv::to_string_bytes`). Backticks (`ShellExecute`) are refused.
    fn compile_composite(&mut self, cs: &CompositeString) -> Result<Reg, Thrown> {
        let parts = match cs {
            CompositeString::Interpolated(i) => &i.parts,
            CompositeString::Document(d) => &d.parts,
            CompositeString::ShellExecute(_) => {
                return Err(Thrown::unsupported(
                    "backtick shell execution",
                    "never spawns a shell; refused by design",
                ))
            }
        };
        // Start from "" so every part is stringified uniformly.
        let dst = self.alloc_temp();
        let empty = self.intern_const(Value::str(""));
        self.emit(Instr::LoadConst { dst, k: empty });
        for part in parts.iter() {
            let pr = match part {
                StringPart::Literal(l) => {
                    let r = self.alloc_temp();
                    let k = self.intern_const(Value::str(l.value.unwrap_or(l.raw).to_vec()));
                    self.emit(Instr::LoadConst { dst: r, k });
                    r
                }
                StringPart::Expression(e) => self.compile_expr(e)?,
                StringPart::BracedExpression(b) => self.compile_expr(b.expression)?,
            };
            self.emit(Instr::Concat { dst, a: dst, b: pr });
        }
        Ok(dst)
    }

    fn compile_constant(&mut self, id: &Identifier) -> Result<Reg, Thrown> {
        let name = identifier_name(id);
        let dst = self.alloc_temp();
        match name {
            b"true" | b"TRUE" | b"True" => self.emit(Instr::LoadBool { dst, b: true }),
            b"false" | b"FALSE" | b"False" => self.emit(Instr::LoadBool { dst, b: false }),
            b"null" | b"NULL" | b"Null" => self.emit(Instr::LoadNull { dst }),
            _ => {
                let n = self.intern_name(name);
                self.emit(Instr::FetchConst { dst, name: n });
            }
        }
        Ok(dst)
    }

    fn compile_construct(&mut self, c: &Construct) -> Result<Reg, Thrown> {
        match c {
            Construct::Print(p) => {
                let src = self.compile_expr(p.value)?;
                let dst = self.alloc_temp();
                self.emit(Instr::Print { dst, src });
                Ok(dst)
            }
            Construct::Eval(_) => Err(Thrown::unsupported(
                "eval()",
                "executes a string as PHP code; refused by design",
            )),
            Construct::Include(_)
            | Construct::IncludeOnce(_)
            | Construct::Require(_)
            | Construct::RequireOnce(_) => Err(Thrown::unsupported(
                "include/require",
                "file inclusion arrives in a later phase",
            )),
            Construct::Isset(i) => {
                // AND over each operand: true iff every value is non-null.
                let dst = self.alloc_temp();
                let mut jumps = Vec::new();
                for v in i.values.iter() {
                    // `isset($o->prop)` must consult `__isset`, not `__get`.
                    if let Expression::Access(Access::Property(pa)) = v {
                        if let ClassLikeMemberSelector::Identifier(li) = &pa.property {
                            let obj = self.compile_expr(pa.object)?;
                            let name = self.intern_name(li.value);
                            let r = self.alloc_temp();
                            self.emit(Instr::IssetProp { dst: r, obj, name });
                            // IssetProp yields a bool; false short-circuits.
                            let pos = self.code.len();
                            self.emit(Instr::Nop); // JmpFalse r -> false
                            jumps.push((pos, r, true));
                            continue;
                        }
                    }
                    let r = self.compile_expr(v)?; // null if absent (DimFetch returns null)
                    let pos = self.code.len();
                    self.emit(Instr::Nop); // JmpNull r -> false
                    jumps.push((pos, r, false));
                }
                self.emit(Instr::LoadBool { dst, b: true });
                let jend = self.code.len();
                self.emit(Instr::Nop); // Jmp end
                let lfalse = self.here();
                self.emit(Instr::LoadBool { dst, b: false });
                let lend = self.here();
                for (pos, r, is_bool) in jumps {
                    self.code[pos] = if is_bool {
                        Instr::JmpFalse { cond: r, target: lfalse }
                    } else {
                        Instr::JmpNull { src: r, target: lfalse }
                    };
                }
                self.code[jend] = Instr::Jmp { target: lend };
                Ok(dst)
            }
            Construct::Empty(e) => {
                // empty(x) == !to_bool(x); absent/null -> true (Not handles it).
                let r = self.compile_expr(e.value)?;
                let dst = self.alloc_temp();
                self.emit(Instr::Not { dst, a: r });
                Ok(dst)
            }
            Construct::Exit(_) | Construct::Die(_) => Err(Thrown::unsupported(
                "exit/die",
                "arrive with the SAPI exit path",
            )),
        }
    }

    fn compile_call(&mut self, call: &phrust_frontend::cst::Call) -> Result<Reg, Thrown> {
        use phrust_frontend::cst::Call;
        match call {
            Call::Function(fc) => self.compile_function_call(fc),
            Call::Method(mc) => {
                self.compile_method_call(mc.object, &mc.method, &mc.argument_list, false)
            }
            Call::StaticMethod(smc) => self.compile_static_call(smc),
            Call::NullSafeMethod(mc) => {
                self.compile_method_call(mc.object, &mc.method, &mc.argument_list, true)
            }
        }
    }

    fn compile_function_call(
        &mut self,
        fc: &phrust_frontend::cst::FunctionCall,
    ) -> Result<Reg, Thrown> {
        let name_bytes = match fc.function {
            Expression::Identifier(id) => identifier_name(id).to_vec(),
            Expression::ConstantAccess(ca) => identifier_name(&ca.name).to_vec(),
            // `$f(...)`, `($expr)(...)`, `$arr[0](...)`: the callee is a runtime
            // value (closure / string name / `[obj, method]` / __invoke object).
            _ => return self.compile_dynamic_call(fc),
        };

        // Spread call `foo(...$args)`: marshal args into an array, emit CallV.
        if has_spread(&fc.argument_list) {
            let args = self.build_args_array(Some(&fc.argument_list))?;
            let name = self.intern_name(&name_bytes);
            let dst = self.alloc_temp();
            self.emit(Instr::CallV { dst, name, args });
            return Ok(dst);
        }

        // Per-parameter by-ref flags of the (statically-known) callee: a user
        // function's recorded signature, or a known by-ref builtin.
        let name_lc = name_bytes.to_ascii_lowercase();
        let sig = self.sigs.get(&name_lc).cloned().or_else(|| builtin_by_ref(&name_lc));

        // Compile each arg into a value reg, or (for a by-ref param) note the
        // variable's CV so it can be materialized as a reference.
        enum ArgSlot {
            Value(Reg),
            Ref(Reg),
        }
        let mut slots = Vec::new();
        for (i, arg) in fc.argument_list.arguments.iter().enumerate() {
            let p = match arg {
                Argument::Positional(p) if p.ellipsis.is_none() => p,
                Argument::Positional(_) => {
                    return Err(Thrown::unsupported("argument spread", "call-site spread is deferred"))
                }
                Argument::Named(_) => {
                    return Err(Thrown::unsupported("named arguments", "deferred"))
                }
            };
            let by_ref = sig.as_ref().and_then(|s| s.get(i).copied()).unwrap_or(false);
            if by_ref {
                let Expression::Variable(Variable::Direct(d)) = p.value else {
                    return Err(Thrown::error(
                        "Error",
                        format!(
                            "{}(): Argument #{} could not be passed by reference",
                            String::from_utf8_lossy(&name_bytes),
                            i + 1
                        ),
                    ));
                };
                slots.push(ArgSlot::Ref(self.cv(strip_dollar(d.name))));
            } else {
                slots.push(ArgSlot::Value(self.compile_expr(p.value)?));
            }
        }
        let argc = slots.len() as u16;
        let base = self.alloc_temps(argc);
        for (i, slot) in slots.into_iter().enumerate() {
            let dst = base + i as Reg;
            match slot {
                ArgSlot::Value(r) => self.emit(Instr::Mov { dst, src: r }),
                ArgSlot::Ref(cv) => self.emit(Instr::MakeRef { dst, cv }),
            }
        }
        let name = self.intern_name(&name_bytes);
        let dst = self.alloc_temp();
        self.emit(Instr::Call { dst, name, base, argc });
        Ok(dst)
    }

    /// `$f(args)` — call a runtime callable value. The callee is evaluated to a
    /// register; arguments are passed by value (the signature is unknown at the
    /// call site, so by-ref parameters are not honoured for dynamic calls).
    fn compile_dynamic_call(
        &mut self,
        fc: &phrust_frontend::cst::FunctionCall,
    ) -> Result<Reg, Thrown> {
        let callee = self.compile_expr(fc.function)?;

        if has_spread(&fc.argument_list) {
            let args = self.build_args_array(Some(&fc.argument_list))?;
            let dst = self.alloc_temp();
            self.emit(Instr::CallDynV { dst, callee, args });
            return Ok(dst);
        }

        let mut argregs = Vec::new();
        for arg in fc.argument_list.arguments.iter() {
            let p = match arg {
                Argument::Positional(p) if p.ellipsis.is_none() => p,
                Argument::Positional(_) => {
                    return Err(Thrown::unsupported("argument spread", "call-site spread is deferred"))
                }
                Argument::Named(_) => return Err(Thrown::unsupported("named arguments", "deferred")),
            };
            argregs.push(self.compile_expr(p.value)?);
        }
        let argc = argregs.len() as u16;
        let base = self.alloc_temps(argc.max(1));
        for (i, r) in argregs.into_iter().enumerate() {
            self.emit(Instr::Mov { dst: base + i as Reg, src: r });
        }
        let dst = self.alloc_temp();
        self.emit(Instr::CallDyn { dst, callee, base, argc });
        Ok(dst)
    }

    /// `fn(params) => expr` — arrow function. Auto-captures (by value) every
    /// free variable of the body that is a CV in the enclosing scope, plus
    /// `$this` when defined and the arrow is non-static.
    fn compile_arrow(
        &mut self,
        af: &phrust_frontend::cst::ArrowFunction,
    ) -> Result<Reg, Thrown> {
        let is_static = af.r#static.is_some();
        // Parameter names, to exclude from auto-capture.
        let params: Vec<Vec<u8>> =
            af.parameter_list.parameters.iter().map(|p| strip_dollar(p.variable.name).to_vec()).collect();

        let mut used = Vec::new();
        collect_vars(af.expression, &mut used);
        let mut captures: Vec<Vec<u8>> = Vec::new();
        for name in used {
            if name == b"this" {
                continue; // handled below
            }
            if params.iter().any(|p| p == &name) || captures.iter().any(|c| c == &name) {
                continue;
            }
            // Only capture variables that exist in the enclosing scope.
            if self.cv_map.iter().any(|n| n == &name) {
                captures.push(name);
            }
        }
        // Bind `$this` by value when we're inside an instance context.
        if !is_static && self.cv_map.iter().any(|n| n == b"this") {
            captures.insert(0, b"this".to_vec());
        }

        self.build_closure(is_static, &af.parameter_list, captures, ClosureBody::Arrow(af.expression))
    }

    /// `function(params) use(...) { body }` — closure with an explicit capture
    /// list. `use(&$x)` by-ref capture is not yet supported (treated by value).
    fn compile_closure(
        &mut self,
        cl: &phrust_frontend::cst::Closure,
    ) -> Result<Reg, Thrown> {
        let is_static = cl.r#static.is_some();
        let mut captures: Vec<Vec<u8>> = Vec::new();
        if !is_static && self.cv_map.iter().any(|n| n == b"this") {
            captures.push(b"this".to_vec());
        }
        if let Some(uses) = &cl.use_clause {
            for u in uses.variables.iter() {
                if u.ampersand.is_some() {
                    return Err(Thrown::unsupported(
                        "by-reference closure capture (`use (&$x)`)",
                        "only by-value `use` is supported for now",
                    ));
                }
                let name = strip_dollar(u.variable.name).to_vec();
                if !captures.iter().any(|c| c == &name) {
                    captures.push(name);
                }
            }
        }
        self.build_closure(is_static, &cl.parameter_list, captures, ClosureBody::Block(&cl.body))
    }

    /// Shared closure lowering: compile the body into a fresh proto whose CVs are
    /// `[captures.., params.., locals..]`, register it, then emit the capture
    /// window + `MakeClosure` in the enclosing frame.
    fn build_closure(
        &mut self,
        is_static: bool,
        param_list: &phrust_frontend::cst::FunctionLikeParameterList,
        captures: Vec<Vec<u8>>,
        body: ClosureBody,
    ) -> Result<Reg, Thrown> {
        let nc = captures.len() as u16;

        // --- compile the closure proto in an isolated builder ---
        let mut c = ProtoBuilder::new(PhpStr::from("{closure}"));
        c.sigs = self.sigs.clone();
        for name in &captures {
            c.cv(name); // captures occupy CVs 0..nc
        }
        let mut params = Vec::new();
        for p in param_list.parameters.iter() {
            let var = strip_dollar(p.variable.name);
            let cv = c.cv(var);
            let name = c.intern_name(var);
            let default = match &p.default_value {
                Some(dv) => Some(c.const_default(dv.value)?),
                None => None,
            };
            params.push(phrust_bytecode::ParamInfo {
                name,
                cv,
                by_ref: p.ampersand.is_some(),
                variadic: p.ellipsis.is_some(),
                default,
            });
        }
        let num_params = params.len() as u16;

        match body {
            ClosureBody::Arrow(e) => c.scan_expr_vars(e),
            ClosureBody::Block(b) => b.statements.iter().for_each(|s| c.scan_stmt_vars(s)),
        }
        c.num_cvs = c.cv_map.len() as u16;
        c.next_temp = c.num_cvs;
        c.high_water = c.num_cvs;
        c.temp_floor = c.num_cvs;

        match body {
            ClosureBody::Arrow(e) => {
                let r = c.compile_expr(e)?;
                c.emit(Instr::Ret { src: r });
            }
            ClosureBody::Block(b) => {
                for s in b.statements.iter() {
                    c.compile_stmt(s)?;
                }
                c.emit(Instr::RetNull);
            }
        }

        let mut proto = c.finish();
        proto.num_params = num_params;
        proto.params = params;
        proto.num_captures = nc;
        proto.is_static = is_static;

        let fnidx = CLOSURES.with(|cls| {
            let mut v = cls.borrow_mut();
            let idx = v.len() as u32;
            v.push(Rc::new(proto));
            idx
        });

        // --- emit the capture window + MakeClosure in the enclosing frame ---
        let base = self.alloc_temps(nc.max(1));
        for (i, name) in captures.iter().enumerate() {
            let dst = base + i as Reg;
            match self.cv_map.iter().position(|n| n == name) {
                Some(src) => self.emit(Instr::Mov { dst, src: src as Reg }),
                None => self.emit(Instr::LoadNull { dst }),
            }
        }
        let dst = self.alloc_temp();
        self.emit(Instr::MakeClosure { dst, fnidx, base, ncaptures: nc });
        Ok(dst)
    }

    /// Collect an lvalue path (root CV + pre-evaluated key/append steps). All
    /// index expressions are compiled into registers here, so the subsequent
    /// cursor opcode sequence is contiguous (no nested cursors).
    fn collect_lvalue_steps(
        &mut self,
        target: &Expression,
        steps: &mut Vec<CursorStep>,
    ) -> Result<LvalRoot, Thrown> {
        match target {
            Expression::Variable(Variable::Direct(d)) => Ok(LvalRoot::Cv(self.cv(strip_dollar(d.name)))),
            Expression::ArrayAccess(aa) => {
                let root = self.collect_lvalue_steps(aa.array, steps)?;
                let key = self.compile_expr(aa.index)?;
                steps.push(CursorStep::Key(key));
                Ok(root)
            }
            Expression::ArrayAppend(ap) => {
                let root = self.collect_lvalue_steps(ap.array, steps)?;
                steps.push(CursorStep::Append);
                Ok(root)
            }
            Expression::Access(Access::Property(pa)) => {
                let root = self.collect_lvalue_steps(pa.object, steps)?;
                let name = self.selector_name(&pa.property)?;
                steps.push(CursorStep::Prop(name));
                Ok(root)
            }
            Expression::Access(Access::StaticProperty(spa)) => {
                // Static property is the cursor root, not a step.
                let class = self.class_ref(spa.class)?;
                let name = self.static_prop_name(&spa.property)?;
                Ok(LvalRoot::StaticProp(class, name))
            }
            _ => Err(Thrown::unsupported(
                "assignment target",
                "only variables, array elements, and properties are supported",
            )),
        }
    }

    fn emit_cursor(&mut self, root: LvalRoot, steps: &[CursorStep]) {
        match root {
            LvalRoot::Cv(cv) => self.emit(Instr::LvalCv { cv }),
            LvalRoot::StaticProp(class, name) => self.emit(Instr::LvalStaticProp { class, name }),
        }
        for step in steps {
            match step {
                CursorStep::Key(r) => self.emit(Instr::LvalKey { key: *r }),
                CursorStep::Append => self.emit(Instr::LvalAppend),
                CursorStep::Prop(n) => self.emit(Instr::LvalProp { name: *n }),
            }
        }
    }

    // --- OOP expression lowering ---

    /// Resolve the class part of `new`/`::`/`instanceof` to a `ClassRef`.
    fn class_ref(&mut self, expr: &Expression) -> Result<ClassRef, Thrown> {
        match expr {
            Expression::Identifier(id) => Ok(ClassRef::Name(self.intern_name(identifier_name(id)))),
            Expression::Self_(_) => Ok(ClassRef::SelfClass),
            Expression::Static(_) => Ok(ClassRef::StaticClass),
            Expression::Parent(_) => Ok(ClassRef::ParentClass),
            other => Ok(ClassRef::Dynamic(self.compile_expr(other)?)),
        }
    }

    fn selector_name(
        &mut self,
        sel: &phrust_frontend::cst::ClassLikeMemberSelector,
    ) -> Result<NIdx, Thrown> {
        match sel {
            ClassLikeMemberSelector::Identifier(li) => Ok(self.intern_name(li.value)),
            _ => Err(Thrown::unsupported("dynamic member name", "`->$x`/`->{expr}` deferred")),
        }
    }

    fn static_prop_name(&mut self, v: &Variable) -> Result<NIdx, Thrown> {
        match v {
            Variable::Direct(d) => Ok(self.intern_name(strip_dollar(d.name))),
            _ => Err(Thrown::unsupported("dynamic static property", "deferred")),
        }
    }

    /// Build a `PhpArray` of the call arguments (used when the call has a
    /// spread `...$x`): positional args are pushed, spreads are merged.
    fn build_args_array(
        &mut self,
        args: Option<&phrust_frontend::cst::ArgumentList>,
    ) -> Result<Reg, Thrown> {
        let dst = self.alloc_temp();
        self.emit(Instr::NewArray { dst, cap: 0 });
        if let Some(list) = args {
            for arg in list.arguments.iter() {
                match arg {
                    Argument::Positional(p) => {
                        let r = self.compile_expr(p.value)?;
                        if p.ellipsis.is_some() {
                            self.emit(Instr::ArrSpread { arr: dst, src: r });
                        } else {
                            self.emit(Instr::ArrPush { arr: dst, val: r });
                        }
                    }
                    Argument::Named(_) => {
                        return Err(Thrown::unsupported("named argument with spread", "deferred"))
                    }
                }
            }
        }
        Ok(dst)
    }

    /// Marshal positional args into a contiguous register window.
    fn compile_arg_window(
        &mut self,
        args: Option<&phrust_frontend::cst::ArgumentList>,
    ) -> Result<(Reg, u16), Thrown> {
        let mut arg_regs = Vec::new();
        if let Some(list) = args {
            for arg in list.arguments.iter() {
                match arg {
                    Argument::Positional(p) if p.ellipsis.is_none() => {
                        arg_regs.push(self.compile_expr(p.value)?)
                    }
                    Argument::Positional(_) => {
                        return Err(Thrown::unsupported("argument spread", "deferred"))
                    }
                    Argument::Named(_) => {
                        return Err(Thrown::unsupported("named arguments", "deferred"))
                    }
                }
            }
        }
        let argc = arg_regs.len() as u16;
        let base = self.alloc_temps(argc);
        for (i, r) in arg_regs.into_iter().enumerate() {
            self.emit(Instr::Mov { dst: base + i as Reg, src: r });
        }
        Ok((base, argc))
    }

    fn compile_new(&mut self, inst: &phrust_frontend::cst::Instantiation) -> Result<Reg, Thrown> {
        let class = self.class_ref(inst.class)?;
        if inst.argument_list.as_ref().is_some_and(has_spread) {
            let args = self.build_args_array(inst.argument_list.as_ref())?;
            let dst = self.alloc_temp();
            self.emit(Instr::NewV { dst, class, args });
            return Ok(dst);
        }
        let (base, argc) = self.compile_arg_window(inst.argument_list.as_ref())?;
        let dst = self.alloc_temp();
        let cache = self.alloc_cache();
        self.emit(Instr::New { dst, class, base, argc, cache });
        Ok(dst)
    }

    fn compile_access(&mut self, acc: &Access) -> Result<Reg, Thrown> {
        match acc {
            Access::Property(pa) => {
                let obj = self.compile_expr(pa.object)?;
                let name = self.selector_name(&pa.property)?;
                let dst = self.alloc_temp();
                let cache = self.alloc_cache();
                self.emit(Instr::FetchProp { dst, obj, name, cache });
                Ok(dst)
            }
            Access::NullSafeProperty(npa) => {
                // `$o?->p`: null short-circuits the whole access to null.
                let obj = self.compile_expr(npa.object)?;
                let dst = self.alloc_temp();
                self.emit(Instr::Mov { dst, src: obj });
                let jn = self.code.len();
                self.emit(Instr::Nop); // JmpNull dst -> Lnull
                let name = self.selector_name(&npa.property)?;
                let cache = self.alloc_cache();
                let t = self.alloc_temp();
                self.emit(Instr::FetchProp { dst: t, obj: dst, name, cache });
                self.emit(Instr::Mov { dst, src: t });
                let jend = self.code.len();
                self.emit(Instr::Nop); // Jmp end
                let lnull = self.here();
                self.emit(Instr::LoadNull { dst });
                let end = self.here();
                self.code[jn] = Instr::JmpNull { src: dst, target: lnull };
                self.code[jend] = Instr::Jmp { target: end };
                Ok(dst)
            }
            Access::StaticProperty(spa) => {
                let class = self.class_ref(spa.class)?;
                let name = self.static_prop_name(&spa.property)?;
                let dst = self.alloc_temp();
                let cache = self.alloc_cache();
                self.emit(Instr::FetchStaticProp { dst, class, name, cache });
                Ok(dst)
            }
            Access::ClassConstant(cca) => {
                let class = self.class_ref(cca.class)?;
                let name = match &cca.constant {
                    phrust_frontend::cst::ClassLikeConstantSelector::Identifier(li) => {
                        self.intern_name(li.value)
                    }
                    _ => return Err(Thrown::unsupported("dynamic class constant", "deferred")),
                };
                let dst = self.alloc_temp();
                let cache = self.alloc_cache();
                self.emit(Instr::FetchClassConst { dst, class, name, cache });
                Ok(dst)
            }
        }
    }

    fn compile_method_call(
        &mut self,
        object: &Expression,
        selector: &phrust_frontend::cst::ClassLikeMemberSelector,
        args: &phrust_frontend::cst::ArgumentList,
        null_safe: bool,
    ) -> Result<Reg, Thrown> {
        let obj = self.compile_expr(object)?;
        if !null_safe {
            let name = self.selector_name(selector)?;
            if has_spread(args) {
                let args = self.build_args_array(Some(args))?;
                let dst = self.alloc_temp();
                self.emit(Instr::CallMethodV { dst, obj, name, args });
                return Ok(dst);
            }
            let (base, argc) = self.compile_arg_window(Some(args))?;
            let dst = self.alloc_temp();
            let cache = self.alloc_cache();
            self.emit(Instr::CallMethod { dst, obj, name, base, argc, cache });
            return Ok(dst);
        }
        // `$o?->m(...)`: if $o is null, the whole call is null and args are not
        // evaluated (the guard jumps over both the arg marshalling and the call).
        let dst = self.alloc_temp();
        self.emit(Instr::Mov { dst, src: obj });
        let jn = self.code.len();
        self.emit(Instr::Nop); // JmpNull dst -> Lnull
        let name = self.selector_name(selector)?;
        let (base, argc) = self.compile_arg_window(Some(args))?;
        let cache = self.alloc_cache();
        let t = self.alloc_temp();
        self.emit(Instr::CallMethod { dst: t, obj: dst, name, base, argc, cache });
        self.emit(Instr::Mov { dst, src: t });
        let jend = self.code.len();
        self.emit(Instr::Nop); // Jmp end
        let lnull = self.here();
        self.emit(Instr::LoadNull { dst });
        let end = self.here();
        self.code[jn] = Instr::JmpNull { src: dst, target: lnull };
        self.code[jend] = Instr::Jmp { target: end };
        Ok(dst)
    }

    fn compile_static_call(
        &mut self,
        smc: &phrust_frontend::cst::StaticMethodCall,
    ) -> Result<Reg, Thrown> {
        let class = self.class_ref(smc.class)?;
        let name = self.selector_name(&smc.method)?;
        if has_spread(&smc.argument_list) {
            let args = self.build_args_array(Some(&smc.argument_list))?;
            let dst = self.alloc_temp();
            self.emit(Instr::CallStaticV { dst, class, name, args });
            return Ok(dst);
        }
        let (base, argc) = self.compile_arg_window(Some(&smc.argument_list))?;
        let dst = self.alloc_temp();
        let cache = self.alloc_cache();
        self.emit(Instr::CallStatic { dst, class, name, base, argc, cache });
        Ok(dst)
    }

    fn compile_assignment(
        &mut self,
        a: &phrust_frontend::cst::Assignment,
    ) -> Result<Reg, Thrown> {
        // Destructuring: `[$a, $b] = ...` / `list($a, $b) = ...`.
        if matches!(a.operator, AssignmentOperator::Assign(_)) {
            if let Expression::Array(arr) = a.lhs {
                let rhs = self.compile_expr(a.rhs)?;
                self.destructure_into(arr.elements.iter(), rhs)?;
                return Ok(rhs);
            }
            if let Expression::List(lst) = a.lhs {
                let rhs = self.compile_expr(a.rhs)?;
                self.destructure_into(lst.elements.iter(), rhs)?;
                return Ok(rhs);
            }
        }

        // Reference assignment `$b =& $a` — rhs is `&<lvalue>`.
        if let (AssignmentOperator::Assign(_), Expression::UnaryPrefix(u)) = (&a.operator, a.rhs) {
            if matches!(u.operator, UnaryPrefixOperator::Reference(_)) {
                let (Expression::Variable(Variable::Direct(dst)), Expression::Variable(Variable::Direct(src))) =
                    (a.lhs, u.operand)
                else {
                    return Err(Thrown::unsupported(
                        "reference binding target",
                        "only `$x =& $y` between simple variables for now",
                    ));
                };
                let dst_cv = self.cv(strip_dollar(dst.name));
                let src_cv = self.cv(strip_dollar(src.name));
                self.emit(Instr::BindRef { dst_cv, src_cv });
                return Ok(dst_cv);
            }
        }

        // Simple single-level `$o->prop = v`: a dedicated opcode so the VM can
        // route undefined/inaccessible writes to `__set` (the cursor can't).
        if let (AssignmentOperator::Assign(_), Expression::Access(Access::Property(pa))) =
            (&a.operator, a.lhs)
        {
            if let ClassLikeMemberSelector::Identifier(li) = &pa.property {
                let obj = self.compile_expr(pa.object)?;
                let name = self.intern_name(li.value);
                let rhs = self.compile_expr(a.rhs)?;
                self.emit(Instr::StoreProp { obj, name, src: rhs });
                return Ok(rhs);
            }
        }

        // Array-element / property / static-property target: use the lvalue cursor.
        if matches!(
            a.lhs,
            Expression::ArrayAccess(_)
                | Expression::ArrayAppend(_)
                | Expression::Access(Access::Property(_))
                | Expression::Access(Access::StaticProperty(_))
        ) {
            let mut steps = Vec::new();
            let root = self.collect_lvalue_steps(a.lhs, &mut steps)?;
            let rhs = self.compile_expr(a.rhs)?;
            return match &a.operator {
                AssignmentOperator::Assign(_) => {
                    self.emit_cursor(root, &steps);
                    self.emit(Instr::AssignLval { src: rhs });
                    Ok(rhs)
                }
                op => {
                    self.emit_cursor(root, &steps);
                    let cur = self.alloc_temp();
                    self.emit(Instr::FetchLvalRw { dst: cur });
                    let res = self.alloc_temp();
                    self.emit(compound_op(op, res, cur, rhs)?);
                    self.emit(Instr::AssignLval { src: res });
                    Ok(res)
                }
            };
        }

        let Expression::Variable(Variable::Direct(d)) = a.lhs else {
            return Err(Thrown::unsupported(
                "assignment target",
                "property targets arrive with OOP",
            ));
        };
        let cv = self.cv(strip_dollar(d.name));
        let rhs = self.compile_expr(a.rhs)?;

        let value_reg = match &a.operator {
            AssignmentOperator::Assign(_) => rhs,
            op => {
                // compound: dst = cv <op> rhs
                let dst = self.alloc_temp();
                let instr = match op {
                    AssignmentOperator::Addition(_) => Instr::Add { dst, a: cv, b: rhs },
                    AssignmentOperator::Subtraction(_) => Instr::Sub { dst, a: cv, b: rhs },
                    AssignmentOperator::Multiplication(_) => Instr::Mul { dst, a: cv, b: rhs },
                    AssignmentOperator::Division(_) => Instr::Div { dst, a: cv, b: rhs },
                    AssignmentOperator::Modulo(_) => Instr::Mod { dst, a: cv, b: rhs },
                    AssignmentOperator::Exponentiation(_) => Instr::Pow { dst, a: cv, b: rhs },
                    AssignmentOperator::Concat(_) => Instr::Concat { dst, a: cv, b: rhs },
                    AssignmentOperator::BitwiseAnd(_) => Instr::BitAnd { dst, a: cv, b: rhs },
                    AssignmentOperator::BitwiseOr(_) => Instr::BitOr { dst, a: cv, b: rhs },
                    AssignmentOperator::BitwiseXor(_) => Instr::BitXor { dst, a: cv, b: rhs },
                    AssignmentOperator::LeftShift(_) => Instr::Shl { dst, a: cv, b: rhs },
                    AssignmentOperator::RightShift(_) => Instr::Shr { dst, a: cv, b: rhs },
                    AssignmentOperator::Coalesce(_) => {
                        // $x ??= rhs : assign only if cv is null
                        self.emit(Instr::Mov { dst, src: cv });
                        let jnn = self.code.len();
                        self.emit(Instr::Nop);
                        self.emit(Instr::Mov { dst, src: rhs });
                        let end = self.code.len() as u32;
                        self.code[jnn] = Instr::JmpNotNull { src: dst, target: end };
                        self.emit(Instr::Assign { cv, src: dst });
                        return Ok(dst);
                    }
                    AssignmentOperator::Assign(_) => unreachable!(),
                };
                self.emit(instr);
                dst
            }
        };
        self.emit(Instr::Assign { cv, src: value_reg });
        Ok(value_reg)
    }
}

// --- free helpers ---------------------------------------------------------

fn strip_dollar(name: &[u8]) -> &[u8] {
    name.strip_prefix(b"$").unwrap_or(name)
}

/// By-reference parameter flags for builtins whose first argument is modified
/// in place (the sort family and the stack/queue ops). The call site then emits
/// a `MakeRef` for arg 0 so the builtin can mutate the caller's variable.
fn builtin_by_ref(name_lc: &[u8]) -> Option<Vec<bool>> {
    let is_ref0 = matches!(
        name_lc,
        b"sort" | b"rsort" | b"asort" | b"arsort" | b"ksort" | b"krsort"
            | b"usort" | b"uasort" | b"uksort"
            | b"array_push" | b"array_pop" | b"array_shift" | b"array_unshift"
            | b"shuffle" | b"array_splice"
    );
    is_ref0.then(|| vec![true])
}

fn has_spread(list: &phrust_frontend::cst::ArgumentList) -> bool {
    list.arguments
        .iter()
        .any(|a| matches!(a, Argument::Positional(p) if p.ellipsis.is_some()))
}

/// Build the instruction for a compound-assignment operator `dst = a <op> b`.
fn compound_op(op: &AssignmentOperator, dst: Reg, a: Reg, b: Reg) -> Result<Instr, Thrown> {
    use AssignmentOperator as Ao;
    Ok(match op {
        Ao::Addition(_) => Instr::Add { dst, a, b },
        Ao::Subtraction(_) => Instr::Sub { dst, a, b },
        Ao::Multiplication(_) => Instr::Mul { dst, a, b },
        Ao::Division(_) => Instr::Div { dst, a, b },
        Ao::Modulo(_) => Instr::Mod { dst, a, b },
        Ao::Exponentiation(_) => Instr::Pow { dst, a, b },
        Ao::Concat(_) => Instr::Concat { dst, a, b },
        Ao::BitwiseAnd(_) => Instr::BitAnd { dst, a, b },
        Ao::BitwiseOr(_) => Instr::BitOr { dst, a, b },
        Ao::BitwiseXor(_) => Instr::BitXor { dst, a, b },
        Ao::LeftShift(_) => Instr::Shl { dst, a, b },
        Ao::RightShift(_) => Instr::Shr { dst, a, b },
        Ao::Coalesce(_) => {
            return Err(Thrown::unsupported(
                "??= on an array element",
                "use a full assignment for now",
            ))
        }
        Ao::Assign(_) => unreachable!("handled by caller"),
    })
}

fn identifier_name<'a>(id: &'a Identifier<'a>) -> &'a [u8] {
    let raw = match id {
        Identifier::Local(l) => l.value,
        Identifier::Qualified(q) => q.value,
        Identifier::FullyQualified(f) => f.value,
    };
    match raw.iter().rposition(|&b| b == b'\\') {
        Some(pos) => &raw[pos + 1..],
        None => raw,
    }
}

/// Fold a compile-time-constant expression (literals, const arrays, true/false/
/// null, negated numeric literals) to a `Value`. Errors on anything requiring
/// runtime evaluation (named constants, class constants, arithmetic).
fn fold_const(e: &Expression) -> Result<Value, Thrown> {
    let v = match e {
        Expression::Literal(l) => literal_value(l),
        Expression::Parenthesized(p) => return fold_const(p.expression),
        Expression::Array(arr) => return fold_const_array(arr.elements.iter()),
        Expression::LegacyArray(arr) => return fold_const_array(arr.elements.iter()),
        Expression::ConstantAccess(ca) => match identifier_name(&ca.name) {
            b"true" | b"TRUE" | b"True" => Value::Bool(true),
            b"false" | b"FALSE" | b"False" => Value::Bool(false),
            b"null" | b"NULL" | b"Null" => Value::Null,
            _ => return Err(non_const_err()),
        },
        Expression::UnaryPrefix(u) => {
            use UnaryPrefixOperator as Op;
            let inner = fold_const(u.operand)?;
            match (&u.operator, inner) {
                (Op::Negation(_), Value::Int(n)) => Value::Int(-n),
                (Op::Negation(_), Value::Float(f)) => Value::Float(-f),
                (Op::Plus(_), v @ (Value::Int(_) | Value::Float(_))) => v,
                _ => return Err(non_const_err()),
            }
        }
        _ => return Err(non_const_err()),
    };
    Ok(v)
}

fn fold_const_array<'a>(
    els: impl Iterator<Item = &'a ArrayElement<'a>>,
) -> Result<Value, Thrown> {
    let mut a = PhpArray::new();
    for el in els {
        match el {
            ArrayElement::KeyValue(kv) => {
                let k = fold_const(kv.key)?;
                let v = fold_const(kv.value)?;
                let key = ArrayKey::from_value(&k)
                    .ok_or_else(|| Thrown::type_error("Illegal offset type"))?;
                a.insert(key, v);
            }
            ArrayElement::Value(v) => a.push(fold_const(v.value)?),
            ArrayElement::Variadic(_) => return Err(non_const_err()),
            ArrayElement::Missing(_) => {}
        }
    }
    Ok(Value::Array(a))
}

/// Extract the class name(s) a `catch` clause's type hint names (`A` or `A|B`).
fn hint_class_names(hint: &Hint) -> Result<Vec<PhpStr>, Thrown> {
    fn collect(hint: &Hint, out: &mut Vec<PhpStr>) -> Result<(), Thrown> {
        match hint {
            Hint::Identifier(id) => {
                out.push(PhpStr::from_bytes(identifier_name(id).to_vec()));
                Ok(())
            }
            Hint::Union(u) => {
                collect(u.left, out)?;
                collect(u.right, out)
            }
            Hint::Parenthesized(p) => collect(p.hint, out),
            _ => Err(Thrown::unsupported("catch type", "only class-name catch types")),
        }
    }
    let mut out = Vec::new();
    collect(hint, &mut out)?;
    Ok(out)
}

fn non_const_err() -> Thrown {
    Thrown::unsupported(
        "non-constant initializer",
        "only literal / array / true/false/null constant initializers for now",
    )
}

fn const_value_static(e: &Expression) -> Result<Value, Thrown> {
    fold_const(e)
}

/// Fold a literal AST node to its runtime value (same rules as the oracle).
fn literal_value(lit: &Literal) -> Value {
    match lit {
        Literal::Integer(i) => match i.value {
            Some(u) if u <= i64::MAX as u64 => Value::Int(u as i64),
            Some(u) => Value::Float(u as f64),
            None => {
                let cleaned: Vec<u8> = i.raw.iter().copied().filter(|&b| b != b'_').collect();
                std::str::from_utf8(&cleaned)
                    .ok()
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(Value::Float)
                    .unwrap_or(Value::Int(0))
            }
        },
        Literal::Float(f) => Value::Float(f.value.0),
        Literal::String(s) => Value::str(s.value.unwrap_or(s.raw).to_vec()),
        Literal::True(_) => Value::Bool(true),
        Literal::False(_) => Value::Bool(false),
        Literal::Null(_) => Value::Null,
    }
}

/// Structural equality for constant-pool deduplication.
fn const_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x.to_bits() == y.to_bits(),
        (Value::Str(x), Value::Str(y)) => x == y,
        _ => false,
    }
}

fn stmt_kind(s: &Statement) -> &'static str {
    match s {
        Statement::If(_) => "if",
        Statement::While(_) => "while",
        Statement::For(_) => "for",
        Statement::Foreach(_) => "foreach",
        Statement::Switch(_) => "switch",
        Statement::Function(_) => "function",
        Statement::Return(_) => "return",
        Statement::Class(_) => "class",
        Statement::Try(_) => "try",
        _ => "this",
    }
}

/// Read-only collection of every `$var` name referenced in an expression, for
/// arrow-function auto-capture. Descends into nested closures/arrows so their
/// free variables propagate upward (over-collection is harmless: the caller
/// keeps only names that are CVs in the enclosing scope). Does not mutate the
/// builder, unlike [`ProtoBuilder::scan_expr_vars`].
fn collect_vars(expr: &Expression, out: &mut Vec<Vec<u8>>) {
    let push = |name: &[u8], out: &mut Vec<Vec<u8>>| {
        if !out.iter().any(|n| n == name) {
            out.push(name.to_vec());
        }
    };
    match expr {
        Expression::Variable(Variable::Direct(d)) => push(strip_dollar(d.name), out),
        Expression::Parenthesized(p) => collect_vars(p.expression, out),
        Expression::Binary(b) => {
            collect_vars(b.lhs, out);
            collect_vars(b.rhs, out);
        }
        Expression::UnaryPrefix(u) => collect_vars(u.operand, out),
        Expression::UnaryPostfix(u) => collect_vars(u.operand, out),
        Expression::Assignment(a) => {
            collect_vars(a.lhs, out);
            collect_vars(a.rhs, out);
        }
        Expression::Conditional(c) => {
            collect_vars(c.condition, out);
            if let Some(t) = &c.then {
                collect_vars(t, out);
            }
            collect_vars(c.r#else, out);
        }
        Expression::Array(arr) => collect_array_vars(arr.elements.iter(), out),
        Expression::LegacyArray(arr) => collect_array_vars(arr.elements.iter(), out),
        Expression::List(lst) => collect_array_vars(lst.elements.iter(), out),
        Expression::ArrayAccess(aa) => {
            collect_vars(aa.array, out);
            collect_vars(aa.index, out);
        }
        Expression::ArrayAppend(ap) => collect_vars(ap.array, out),
        Expression::CompositeString(cs) => {
            let parts = match cs {
                CompositeString::Interpolated(i) => Some(&i.parts),
                CompositeString::Document(d) => Some(&d.parts),
                CompositeString::ShellExecute(_) => None,
            };
            if let Some(parts) = parts {
                for part in parts.iter() {
                    match part {
                        StringPart::Expression(e) => collect_vars(e, out),
                        StringPart::BracedExpression(b) => collect_vars(b.expression, out),
                        StringPart::Literal(_) => {}
                    }
                }
            }
        }
        Expression::Match(m) => {
            collect_vars(m.expression, out);
            for arm in m.arms.iter() {
                match arm {
                    MatchArm::Expression(a) => {
                        a.conditions.iter().for_each(|c| collect_vars(c, out));
                        collect_vars(a.expression, out);
                    }
                    MatchArm::Default(a) => collect_vars(a.expression, out),
                }
            }
        }
        Expression::Instantiation(inst) => {
            collect_vars(inst.class, out);
            if let Some(list) = &inst.argument_list {
                collect_arg_vars(list, out);
            }
        }
        Expression::Access(acc) => match acc {
            Access::Property(pa) => collect_vars(pa.object, out),
            Access::NullSafeProperty(pa) => collect_vars(pa.object, out),
            Access::StaticProperty(spa) => collect_vars(spa.class, out),
            Access::ClassConstant(cca) => collect_vars(cca.class, out),
        },
        Expression::Clone(cl) => collect_vars(cl.object, out),
        Expression::Throw(t) => collect_vars(t.exception, out),
        Expression::Construct(c) => match c {
            Construct::Print(p) => collect_vars(p.value, out),
            Construct::Empty(e) => collect_vars(e.value, out),
            Construct::Isset(i) => i.values.iter().for_each(|v| collect_vars(v, out)),
            _ => {}
        },
        Expression::Call(call) => {
            use phrust_frontend::cst::Call;
            match call {
                Call::Function(fc) => {
                    collect_vars(fc.function, out);
                    collect_arg_vars(&fc.argument_list, out);
                }
                Call::Method(mc) => {
                    collect_vars(mc.object, out);
                    collect_arg_vars(&mc.argument_list, out);
                }
                Call::NullSafeMethod(mc) => {
                    collect_vars(mc.object, out);
                    collect_arg_vars(&mc.argument_list, out);
                }
                Call::StaticMethod(smc) => {
                    collect_vars(smc.class, out);
                    collect_arg_vars(&smc.argument_list, out);
                }
            }
        }
        // Nested closure/arrow: only the vars it needs from *our* scope matter.
        Expression::ArrowFunction(af) => collect_vars(af.expression, out),
        Expression::Closure(cl) => {
            if let Some(uses) = &cl.use_clause {
                for u in uses.variables.iter() {
                    push(strip_dollar(u.variable.name), out);
                }
            }
        }
        _ => {}
    }
}

fn collect_arg_vars(list: &phrust_frontend::cst::ArgumentList, out: &mut Vec<Vec<u8>>) {
    for arg in list.arguments.iter() {
        match arg {
            Argument::Positional(p) => collect_vars(p.value, out),
            Argument::Named(n) => collect_vars(n.value, out),
        }
    }
}

fn collect_array_vars<'a>(
    els: impl Iterator<Item = &'a ArrayElement<'a>>,
    out: &mut Vec<Vec<u8>>,
) {
    for el in els {
        match el {
            ArrayElement::KeyValue(kv) => {
                collect_vars(kv.key, out);
                collect_vars(kv.value, out);
            }
            ArrayElement::Value(v) => collect_vars(v.value, out),
            ArrayElement::Variadic(v) => collect_vars(v.value, out),
            ArrayElement::Missing(_) => {}
        }
    }
}

fn expr_kind(e: &Expression) -> &'static str {
    match e {
        Expression::CompositeString(_) => "interpolated-string",
        Expression::Match(_) => "match",
        Expression::Closure(_) => "closure",
        Expression::ArrayAccess(_) => "array-access",
        Expression::Throw(_) => "throw",
        _ => "this",
    }
}

// FnIdx is reserved for user-function references (phase 5+).
const _: fn() = || {
    let _ = FnIdx::default;
};
