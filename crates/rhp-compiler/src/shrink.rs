//! Dropping the growth slack of a finished program.

use std::rc::Rc;

use rhp_bytecode::{
    AttributeProto, ClassProto, ConstInitProto, FunctionProto, Program, PropDefaultProto, TypeHint,
};

/// Drop the growth slack of a compiled program's vectors (the protos' own
/// instruction and constant vectors are shrunk as each is finished): the
/// VM keeps the declarations as compiled for as long as the program lives,
/// with opcache across requests. Shared parts (the prelude's) are left.
pub(crate) fn shrink_program(p: &mut Program) {
    shrink_fn(&mut p.main);
    for f in p.functions.iter_mut().chain(p.closures.iter_mut()) {
        shrink_fn(f);
    }
    for c in p.classes.iter_mut().chain(p.cond_classes.iter_mut()) {
        shrink_class(c);
    }
}

fn shrink_fn(f: &mut Rc<FunctionProto>) {
    let Some(f) = Rc::get_mut(f) else {
        return;
    };
    f.params.shrink_to_fit();
    for p in &mut f.params {
        if let Some(t) = &mut p.type_hint {
            shrink_type(t);
        }
        shrink_meta_attrs(&mut p.attributes);
        if let Some(t) = &mut p.default_thunk {
            shrink_fn(t);
        }
    }
    if let Some(t) = &mut f.return_type {
        shrink_type(t);
    }
    f.cv_var_names.shrink_to_fit();
    shrink_meta_attrs(&mut f.meta.attributes);
}

fn shrink_type(t: &mut TypeHint) {
    t.kinds.shrink_to_fit();
    t.groups.shrink_to_fit();
    for g in &mut t.groups {
        g.shrink_to_fit();
    }
}

fn shrink_meta_attrs(attrs: &mut Vec<AttributeProto>) {
    attrs.shrink_to_fit();
    for a in attrs {
        if let Some(i) = &mut a.args {
            shrink_init(i);
        }
    }
}

fn shrink_init(i: &mut ConstInitProto) {
    if let ConstInitProto::Thunk(t) = i {
        shrink_fn(t);
    }
}

fn shrink_class(c: &mut Rc<ClassProto>) {
    let Some(c) = Rc::get_mut(c) else {
        return;
    };
    c.interfaces.shrink_to_fit();
    c.traits.shrink_to_fit();
    c.methods.shrink_to_fit();
    for m in &mut c.methods {
        shrink_fn(&mut m.proto);
    }
    c.props.shrink_to_fit();
    for p in &mut c.props {
        if let PropDefaultProto::Init(t) = &mut p.default {
            shrink_fn(t);
        }
        if let Some(t) = &mut p.type_hint {
            shrink_type(t);
        }
        shrink_meta_attrs(&mut p.meta.attributes);
    }
    c.static_props.shrink_to_fit();
    for sp in &mut c.static_props {
        shrink_init(&mut sp.init);
        if let Some(t) = &mut sp.type_hint {
            shrink_type(t);
        }
        shrink_meta_attrs(&mut sp.meta.attributes);
    }
    c.constants.shrink_to_fit();
    for k in &mut c.constants {
        shrink_init(&mut k.value);
        if let Some(t) = &mut k.type_hint {
            shrink_type(t);
        }
        shrink_meta_attrs(&mut k.meta.attributes);
    }
    c.enum_cases.shrink_to_fit();
    for e in &mut c.enum_cases {
        if let Some(i) = &mut e.value {
            shrink_init(i);
        }
        shrink_meta_attrs(&mut e.meta.attributes);
    }
    shrink_meta_attrs(&mut c.meta.attributes);
}
