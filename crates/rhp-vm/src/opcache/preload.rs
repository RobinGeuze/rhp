//! `opcache.preload` (ext/opcache/ZendAccelerator.c, `accel_preload`): a
//! script run once per thread before its first request. The functions and
//! classes it declares are recorded as (file, declaration) origins and
//! declared again in every request, before the request's script; its
//! globals and constants are dropped, as in php.

use std::rc::Rc;

use rhp_builtins as builtins;
use rhp_bytecode::Program;
use rhp_values::{ArrayKey, PhpArray, Value};

use super::{ini_flag, now, os_path, path_bytes, program_size, with_cache, Cache, Request, Script};
use crate::{IncludeError, Vm};

/// A file compiled while preloading, and where the preload run's engine
/// installed it (its closure, class-declaration and function table bases).
pub(crate) struct PreloadRec {
    path: Vec<u8>,
    prog: Rc<Program>,
    /// The preload script itself, compiled as a main script.
    main: bool,
    closure_base: usize,
    decl_base: usize,
    fn_base: usize,
    fn_count: usize,
}

impl PreloadRec {
    pub(crate) fn new(
        path: &[u8],
        prog: Rc<Program>,
        main: bool,
        (closure_base, decl_base, fn_base): (usize, usize, usize),
        fn_count: usize,
    ) -> PreloadRec {
        PreloadRec {
            path: path.to_vec(),
            prog,
            main,
            closure_base,
            decl_base,
            fn_base,
            fn_count,
        }
    }

    /// The program's class declarations (top-level, nested, anonymous).
    fn decl_count(&self) -> usize {
        let (top, nested, anon) = crate::classes::decl_layout(&self.prog);
        top + nested + anon
    }

    /// The table slots of its top-level and anonymous class declarations.
    fn top_slots(&self) -> impl Iterator<Item = usize> {
        let (top, nested, anon) = crate::classes::decl_layout(&self.prog);
        let b = self.decl_base;
        (b..b + top).chain(b + top + nested..b + top + nested + anon)
    }
}

/// Where a preloaded function comes from in its file's include program.
#[derive(Clone, Copy)]
enum FnOrigin {
    /// A top-level function: its index in `functions`.
    Top(usize),
    /// A conditional one (`DeclareFunc`): its index in `closures`.
    Closure(usize),
}

/// What `opcache.preload` declared (php's `$PRELOAD$` script), installed in
/// every request before its script runs. Only declarations are kept: the
/// preload script's globals and constants are gone, as in php.
pub(crate) struct Preload {
    /// The files compiled while preloading, in order, as include programs.
    programs: Vec<(Vec<u8>, Rc<Program>)>,
    /// Functions to declare, in declaration order: (program, origin).
    functions: Vec<(usize, FnOrigin)>,
    /// Classes to link, in link order: (program, class declaration).
    classes: Vec<(usize, usize)>,
    /// `class_alias()`es of preloaded classes: (lowercase alias, index into
    /// `classes`).
    aliases: Vec<(Vec<u8>, usize)>,
    fn_names: std::collections::HashSet<Vec<u8>>,
    memory: i64,
}

impl Preload {
    pub(crate) fn has_file(&self, path: &[u8]) -> bool {
        !path.is_empty() && self.programs.iter().any(|(p, _)| p == path)
    }

    pub(crate) fn has_function(&self, lcname: &[u8]) -> bool {
        self.fn_names.contains(lcname)
    }
}

impl Vm {
    /// The `opcache.preload` script this thread still has to run, if any:
    /// opcache is started and preloading wasn't tried yet. Call on a fresh
    /// engine with the SAPI and ini settings of the request about to run.
    pub fn preload_pending(&self) -> Option<String> {
        let startup = |n: &[u8]| builtins::ini_startup_value(n).unwrap_or_default();
        let cli = builtins::sapi::sapi_name() == "cli";
        let started = ini_flag(&startup(b"opcache.enable"))
            && (!cli || ini_flag(&startup(b"opcache.enable_cli")));
        let script = startup(b"opcache.preload");
        if !started || script.is_empty() || Vm::preload_outcome().is_some() {
            return None;
        }
        Some(String::from_utf8_lossy(&script).into_owned())
    }

    /// Whether `opcache.preload` succeeded on this thread, once it ran.
    /// After a failure php doesn't start, so a server should fail every
    /// later request on the thread too.
    pub fn preload_outcome() -> Option<bool> {
        with_cache(|c| c.preload_result)
    }

    /// Run the preload script in this fresh engine (php's `accel_preload`):
    /// compile and run it, link the classes of files it only compiled
    /// (`opcache_compile_file`), and keep the functions and classes it
    /// declared for every later request on this thread. Returns false when
    /// it failed (a parse error, an uncaught exception, a fatal error):
    /// php then doesn't start.
    pub fn run_preload(
        &mut self,
        path: &str,
        compile: impl FnOnce(&[u8]) -> Result<Program, IncludeError>,
    ) -> bool {
        // A failure below leaves it failed.
        with_cache(|c| c.preload_result = Some(false));
        builtins::preload_user_check();
        // Compiles bypass the cache while preloading (php's ZCG(enabled) = 0).
        self.opcache = Some(Request {
            started: true,
            enabled: false,
            request_time: now(),
        });
        let real = std::fs::canonicalize(path)
            .map(|p| path_bytes(&p))
            .unwrap_or_else(|_| path.as_bytes().to_vec());
        let source = match std::fs::read(os_path(&real)) {
            Ok(s) => s,
            Err(e) => {
                // The preload script is required at startup.
                let reason = builtins::streams::strerror(e.raw_os_error().unwrap_or(2));
                let warning = format!("PHP Startup: Failed to open stream: {reason}");
                self.display_error(2, "Warning", warning.as_bytes(), b"Unknown", 0);
                let ip = builtins::Host::ini_get(self, b"include_path").unwrap_or_default();
                let msg = format!(
                    "Failed opening required '{path}' (include_path='{}')",
                    String::from_utf8_lossy(&ip)
                );
                self.compile_error(&msg, b"Unknown", 0);
                return false;
            }
        };
        let file = String::from_utf8_lossy(&real).into_owned();
        let prog = match compile(&source) {
            Ok(p) => Rc::new(p),
            Err(IncludeError::Parse {
                class,
                message,
                line,
            }) => {
                self.report_parse_error(class, &message, &file, line);
                return false;
            }
            Err(IncludeError::Compile { message, line }) => {
                self.compile_error(&message, file.as_bytes(), line);
                return false;
            }
            Err(IncludeError::Thrown(_)) => return false,
        };
        self.preload_rec = Some(vec![PreloadRec::new(
            &real,
            prog.clone(),
            true,
            (0, 0, 0),
            prog.functions.len(),
        )]);
        self.set_main_script_path(real.clone());
        let ok = matches!(self.run_main(&prog), Ok(0));
        let recs = self.preload_rec.take().unwrap_or_default();
        if !ok {
            return false;
        }
        self.preload_link(&recs);
        let Some(preload) = self.build_preload(&recs, &source) else {
            return false;
        };
        let preload = Rc::new(preload);
        // The preloaded files are cached for good (timestamp 0: never
        // revalidated), under a `$PRELOAD$` pseudo-script.
        self.opcache = None;
        with_cache(|c| {
            c.preload = Some(preload.clone());
            c.preload_result = Some(true);
            cache_preloaded(c, &preload, Some(prog), now());
        });
        true
    }

    /// `preload_link`: link the top-level classes the preload run declared
    /// no statement for (files it only compiled), in passes until nothing
    /// changes; warn about the ones that still can't be linked.
    fn preload_link(&mut self, recs: &[PreloadRec]) {
        let slots: Vec<usize> = recs.iter().flat_map(|r| r.top_slots()).collect();
        let pending = |vm: &Vm, slot: usize| {
            let (proto, done) = &vm.decl_classes[slot];
            let anon = crate::classes::is_anon_name(proto.name.as_bytes());
            !*done
                && (anon
                    || !vm
                        .class_ids
                        .contains_key(&proto.name.as_bytes().to_ascii_lowercase()))
        };
        let mut errors: std::collections::HashMap<usize, (String, Vec<u8>, i64)> =
            std::collections::HashMap::new();
        loop {
            let mut changed = false;
            for &slot in &slots {
                if !pending(self, slot) || !self.deps_ready(&self.decl_classes[slot].0) {
                    continue;
                }
                let proto = self.decl_classes[slot].0.clone();
                match self.link_slot(slot, &proto) {
                    Ok(()) => {
                        self.decl_classes[slot].1 = true;
                        changed = true;
                    }
                    Err(t) => {
                        let msg = self.thrown_message(t);
                        let file = proto.decl_file.as_bytes().to_vec();
                        errors.insert(slot, (msg, file, proto.decl_line as i64));
                    }
                }
            }
            if !changed {
                break;
            }
        }
        for &slot in &slots {
            let (proto, done) = &self.decl_classes[slot];
            if *done {
                continue;
            }
            let proto = proto.clone();
            // A C string: an anonymous class's name stops at its NUL.
            let raw = proto.name.as_bytes();
            let shown = &raw[..raw.iter().position(|&c| c == 0).unwrap_or(raw.len())];
            let name = String::from_utf8_lossy(shown).into_owned();
            let file = proto.decl_file.as_bytes().to_vec();
            let line = proto.decl_line as i64;
            let anon = crate::classes::is_anon_name(proto.name.as_bytes());
            let known = |n: &rhp_values::PhpStr| {
                self.class_ids
                    .contains_key(&n.as_bytes().to_ascii_lowercase())
            };
            let missing = proto
                .parent
                .iter()
                .filter(|n| !known(n))
                .map(|n| ("Unknown parent ", n))
                .chain(
                    proto
                        .interfaces
                        .iter()
                        .filter(|n| !known(n))
                        .map(|n| ("Unknown interface ", n)),
                )
                .chain(
                    proto
                        .traits
                        .iter()
                        .filter(|n| !known(n))
                        .map(|n| ("Unknown trait ", n)),
                )
                .next()
                .map(|(k, n)| format!("{k}{}", String::from_utf8_lossy(n.as_bytes())));
            let (msg, file, line) = if !anon
                && self
                    .class_ids
                    .contains_key(&proto.name.as_bytes().to_ascii_lowercase())
            {
                (
                    format!("Can't preload already declared class {name}"),
                    file,
                    line,
                )
            } else if let Some(m) = missing {
                (
                    format!("Can't preload unlinked class {name}: {m}"),
                    file,
                    line,
                )
            } else if let Some((m, f, l)) = errors.remove(&slot) {
                (format!("Can't preload unlinked class {name}: {m}"), f, l)
            } else {
                continue;
            };
            self.display_error(2, "Warning", msg.as_bytes(), &file, line);
        }
    }

    /// An inheritance error's message (the preload run displays it in its
    /// "Can't preload" warning instead).
    fn thrown_message(&mut self, t: rhp_values::Thrown) -> String {
        match t {
            rhp_values::Thrown::Error { message, .. } => message,
            other => match self.materialize(other, None) {
                rhp_values::Thrown::Object(o) => {
                    let m = o
                        .borrow()
                        .properties
                        .get(&rhp_values::PhpStr::from("message"))
                        .cloned()
                        .unwrap_or(Value::Null);
                    String::from_utf8_lossy(&rhp_values::conv::to_string_bytes(&m)).into_owned()
                }
                rhp_values::Thrown::Error { message, .. } => message,
                _ => String::new(),
            },
        }
    }

    /// What the preload run declared, as program-relative origins.
    fn build_preload(&mut self, recs: &[PreloadRec], main_source: &[u8]) -> Option<Preload> {
        let main = recs.first()?;
        // The preload script as an include program: same user code, without
        // the prelude (whose closures come first in the main compile).
        let main_file = String::from_utf8_lossy(&main.path).into_owned();
        let mut cb = self.includer.take()?;
        let inc = cb(&main_file, main_source);
        self.includer = Some(cb);
        let inc = Rc::new(inc.ok()?);
        let closure_skew = main.prog.closures.len().checked_sub(inc.closures.len())?;
        let fn_skew = main.prog.num_prelude_functions;
        let mut programs: Vec<(Vec<u8>, Rc<Program>)> = Vec::new();
        for r in recs {
            let p = if r.main { inc.clone() } else { r.prog.clone() };
            programs.push((r.path.clone(), p));
        }
        let find = |pred: &dyn Fn(&PreloadRec) -> bool| recs.iter().position(pred);
        let mut functions = Vec::new();
        let mut fn_names = std::collections::HashSet::new();
        for i in fn_skew..self.functions.len() {
            let f = &self.functions[i];
            let origin = match self.closures.iter().position(|c| Rc::ptr_eq(c, f)) {
                Some(abs) => {
                    let Some(r) = find(&|r| {
                        abs >= r.closure_base && abs < r.closure_base + r.prog.closures.len()
                    }) else {
                        continue;
                    };
                    let mut local = abs - recs[r].closure_base;
                    if recs[r].main {
                        let Some(l) = local.checked_sub(closure_skew) else {
                            continue;
                        };
                        local = l;
                    }
                    (r, FnOrigin::Closure(local))
                }
                None => {
                    let Some(r) = find(&|r| i >= r.fn_base && i < r.fn_base + r.fn_count) else {
                        continue;
                    };
                    let mut local = i - recs[r].fn_base;
                    if recs[r].main {
                        local -= fn_skew;
                    }
                    (r, FnOrigin::Top(local))
                }
            };
            fn_names.insert(f.name.as_bytes().to_ascii_lowercase());
            functions.push(origin);
        }
        // preload_move_user_functions takes them last declared first.
        functions.reverse();
        let mut classes = Vec::new();
        let mut class_index = std::collections::HashMap::new();
        for cid in self.internal_classes..self.classes.len() {
            let Some(&slot) = self.class_order.get(cid) else {
                continue;
            };
            if let Some(r) = find(&|r| slot >= r.decl_base && slot < r.decl_base + r.decl_count()) {
                class_index.insert(cid as u32, classes.len());
                classes.push((r, slot - recs[r].decl_base));
            }
        }
        let mut aliases: Vec<(Vec<u8>, usize)> = self
            .class_ids
            .iter()
            .filter_map(|(name, cid)| {
                let i = *class_index.get(cid)?;
                let own = self.classes[*cid as usize]
                    .name
                    .as_bytes()
                    .to_ascii_lowercase();
                (*name != own).then(|| (name.clone(), i))
            })
            .collect();
        aliases.sort();
        let memory = programs.iter().map(|(_, p)| program_size(p)).sum::<i64>() / 4;
        Some(Preload {
            programs,
            functions,
            classes,
            aliases,
            fn_names,
            memory,
        })
    }

    /// Declare this thread's preloaded functions and classes in this
    /// request (php's `preload_load`), before its script is compiled in.
    pub(crate) fn install_preload(&mut self) {
        let Some(pre) = with_cache(|c| c.preload.clone()) else {
            return;
        };
        if self.preload_rec.is_some() || !self.opcache_request().started {
            return;
        }
        let mut bases = Vec::new();
        for (_, prog) in &pre.programs {
            let base = self.closures.len() as u32;
            let cbase = self.decl_classes.len();
            for c in &prog.closures {
                self.closures.push(Rc::new(crate::include::rebase_proto_c(
                    c,
                    base,
                    cbase as u32,
                )));
            }
            self.register_decl_classes(prog, base);
            bases.push((base, cbase));
        }
        for &(p, origin) in &pre.functions {
            let (base, cbase) = bases[p];
            let f = match origin {
                FnOrigin::Top(i) => Rc::new(crate::include::rebase_proto_c(
                    &pre.programs[p].1.functions[i],
                    base,
                    cbase as u32,
                )),
                FnOrigin::Closure(i) => self.closures[base as usize + i].clone(),
            };
            let lc = f.name.as_bytes().to_ascii_lowercase();
            if self.fn_names.contains_key(&lc) {
                continue;
            }
            self.fn_names.insert(lc, self.functions.len());
            self.functions.push(f);
        }
        // Linking ran (and warned) while preloading; replaying it is quiet.
        let last_error = self.last_error.clone();
        self.silence += 1;
        let mut linked = vec![None; pre.classes.len()];
        for (i, &(p, local)) in pre.classes.iter().enumerate() {
            let slot = bases[p].1 + local;
            let proto = self.decl_classes[slot].0.clone();
            let anon = crate::classes::is_anon_name(proto.name.as_bytes());
            if !anon
                && self
                    .class_ids
                    .contains_key(&proto.name.as_bytes().to_ascii_lowercase())
            {
                continue;
            }
            let id = self.classes.len() as u32;
            if self.link_slot(slot, &proto).is_ok() {
                self.decl_classes[slot].1 = true;
                linked[i] = Some(id);
            }
        }
        for (alias, i) in &pre.aliases {
            if let Some(id) = linked[*i] {
                self.class_ids.entry(alias.clone()).or_insert(id);
            }
        }
        self.silence -= 1;
        self.last_error = last_error;
        self.preloaded = Some(pre);
    }

    /// A preloaded file included at run time: its class declarations whose
    /// class preloading declared are already done (php removed them).
    pub(crate) fn skip_preloaded_decls(&mut self, cbase: usize) {
        for slot in cbase..self.decl_classes.len() {
            let name = self.decl_classes[slot]
                .0
                .name
                .as_bytes()
                .to_ascii_lowercase();
            if self.class_ids.contains_key(&name) {
                self.decl_classes[slot].1 = true;
            }
        }
    }
}

/// `opcache_get_status()['preload_statistics']`.
/// Put the preloaded files in the cache (php's `preload_restart` puts them
/// back after every restart): never revalidated (timestamp 0), plus the
/// `$PRELOAD$` pseudo-script holding their declarations. `main` is the
/// preload script's main compile, kept the first time.
pub(super) fn cache_preloaded(c: &mut Cache, pre: &Preload, main: Option<Rc<Program>>, time: i64) {
    let entry = |c: &mut Cache, main: Option<Rc<Program>>, include: Option<Rc<Program>>, memory| {
        c.used += memory;
        c.seq += 1;
        Script {
            main,
            include,
            timestamp: 0,
            revalidate: 0,
            hits: 0,
            last_used: time,
            memory,
            corrupted: false,
            seq: c.seq,
        }
    };
    let mut main = main;
    for (path, p) in pre.programs.iter().filter(|(path, _)| !path.is_empty()) {
        let s = entry(c, main.take(), Some(p.clone()), program_size(p));
        c.scripts.insert(path.clone(), s);
    }
    let s = entry(c, None, None, pre.memory);
    c.scripts.insert(b"$PRELOAD$".to_vec(), s);
}

pub(super) fn preload_report(pre: &Preload) -> PhpArray {
    let s = |b: &[u8]| ArrayKey::from_bytes(b);
    let mut out = PhpArray::new();
    out.insert(s(b"memory_consumption"), Value::Int(pre.memory));
    let mut fns = PhpArray::new();
    for &(p, origin) in &pre.functions {
        let prog = &pre.programs[p].1;
        let f = match origin {
            FnOrigin::Top(i) => &prog.functions[i],
            FnOrigin::Closure(i) => &prog.closures[i],
        };
        fns.push(Value::str(f.name.as_bytes().to_vec()));
    }
    if !fns.is_empty() {
        out.insert(s(b"functions"), Value::Array(fns));
    }
    let mut classes = PhpArray::new();
    for &(p, local) in &pre.classes {
        let prog = &pre.programs[p].1;
        // The declaration table's order: top-level, nested, anonymous.
        let user = &prog.classes[prog.num_prelude_classes..];
        let (top, nested, _) = crate::classes::decl_layout(prog);
        let c = if local < top {
            user.get(local)
        } else if local < top + nested {
            prog.cond_classes.get(local - top)
        } else {
            user.get(local - nested)
        };
        if let Some(c) = c {
            classes.push(Value::str(c.name.as_bytes().to_vec()));
        }
    }
    if !classes.is_empty() {
        out.insert(s(b"classes"), Value::Array(classes));
    }
    let mut scripts = PhpArray::new();
    for (path, _) in pre.programs.iter().filter(|(path, _)| !path.is_empty()) {
        scripts.push(Value::str(path.clone()));
    }
    out.insert(s(b"scripts"), Value::Array(scripts));
    out
}
