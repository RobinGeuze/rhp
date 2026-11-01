//! XML Schema 1.0 validation (libxml2's `xmlschemas.c` as php's
//! `DOMDocument::schemaValidate*` drives it): element / attribute
//! declarations, named and anonymous complex & simple types, sequence /
//! choice / all / group references, attribute groups, wildcards,
//! substitution groups, simple content, enumerations, unions and the
//! common built-in datatypes — reporting libxml2's error codes and wording.
//!
//! Content models compile to a Thompson NFA; the "Expected is ( … )" lists
//! are the labels leaving the current state set, in schema order.

use std::collections::HashMap;

use super::parser::{self, ParseOpts, XmlError};
use super::tree::{Doc, Kind, NodeId};

pub const XSD_NS: &[u8] = b"http://www.w3.org/2001/XMLSchema";
pub const XSI_NS: &[u8] = b"http://www.w3.org/2001/XMLSchema-instance";

type TypeId = usize;
type ElemId = usize;
type QName = (Option<Vec<u8>>, Vec<u8>);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Builtin {
    AnyType,
    AnySimpleType,
    String,
    NormalizedString,
    Token,
    Language,
    Name,
    NCName,
    Id,
    IdRef,
    IdRefs,
    Entity,
    NmToken,
    NmTokens,
    QName,
    Boolean,
    Decimal,
    Integer,
    NonPositiveInteger,
    NegativeInteger,
    Long,
    Int,
    Short,
    Byte,
    NonNegativeInteger,
    UnsignedLong,
    UnsignedInt,
    UnsignedShort,
    UnsignedByte,
    PositiveInteger,
    Float,
    Double,
    AnyUri,
    Date,
    DateTime,
    Time,
    Duration,
    GYear,
    GYearMonth,
    GMonth,
    GMonthDay,
    GDay,
    Base64Binary,
    HexBinary,
    Notation,
}

const BUILTINS: &[(&str, Builtin)] = &[
    ("anyType", Builtin::AnyType),
    ("anySimpleType", Builtin::AnySimpleType),
    ("string", Builtin::String),
    ("normalizedString", Builtin::NormalizedString),
    ("token", Builtin::Token),
    ("language", Builtin::Language),
    ("Name", Builtin::Name),
    ("NCName", Builtin::NCName),
    ("ID", Builtin::Id),
    ("IDREF", Builtin::IdRef),
    ("IDREFS", Builtin::IdRefs),
    ("ENTITY", Builtin::Entity),
    ("ENTITIES", Builtin::IdRefs),
    ("NMTOKEN", Builtin::NmToken),
    ("NMTOKENS", Builtin::NmTokens),
    ("QName", Builtin::QName),
    ("boolean", Builtin::Boolean),
    ("decimal", Builtin::Decimal),
    ("integer", Builtin::Integer),
    ("nonPositiveInteger", Builtin::NonPositiveInteger),
    ("negativeInteger", Builtin::NegativeInteger),
    ("long", Builtin::Long),
    ("int", Builtin::Int),
    ("short", Builtin::Short),
    ("byte", Builtin::Byte),
    ("nonNegativeInteger", Builtin::NonNegativeInteger),
    ("unsignedLong", Builtin::UnsignedLong),
    ("unsignedInt", Builtin::UnsignedInt),
    ("unsignedShort", Builtin::UnsignedShort),
    ("unsignedByte", Builtin::UnsignedByte),
    ("positiveInteger", Builtin::PositiveInteger),
    ("float", Builtin::Float),
    ("double", Builtin::Double),
    ("anyURI", Builtin::AnyUri),
    ("date", Builtin::Date),
    ("dateTime", Builtin::DateTime),
    ("time", Builtin::Time),
    ("duration", Builtin::Duration),
    ("gYear", Builtin::GYear),
    ("gYearMonth", Builtin::GYearMonth),
    ("gMonth", Builtin::GMonth),
    ("gMonthDay", Builtin::GMonthDay),
    ("gDay", Builtin::GDay),
    ("base64Binary", Builtin::Base64Binary),
    ("hexBinary", Builtin::HexBinary),
    ("NOTATION", Builtin::Notation),
];

#[derive(Clone, Debug)]
enum Variety {
    Atomic,
    List(TypeId),
    Union(Vec<TypeId>),
}

#[derive(Clone, Debug)]
struct SimpleType {
    name: Option<QName>,
    builtin: Builtin,
    variety: Variety,
    base: Option<TypeId>,
    enums: Vec<Vec<u8>>,
    min_len: Option<usize>,
    max_len: Option<usize>,
    len: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Process {
    Skip,
    Lax,
    Strict,
}

#[derive(Clone, Debug)]
enum WNs {
    Any,
    Other(Option<Vec<u8>>),
    List(Vec<Option<Vec<u8>>>),
}

#[derive(Clone, Debug)]
struct Wildcard {
    ns: WNs,
    process: Process,
}

impl Wildcard {
    fn allows(&self, ns: &Option<Vec<u8>>) -> bool {
        match &self.ns {
            WNs::Any => true,
            WNs::Other(t) => ns.is_some() && ns != t,
            WNs::List(l) => l.contains(ns),
        }
    }
}

#[derive(Clone, Debug)]
struct AttrUse {
    name: QName,
    ty: TypeId,
    required: bool,
    prohibited: bool,
}

#[derive(Clone, Debug)]
enum Term {
    Elem(ElemId),
    Any(Wildcard),
    Seq(Vec<Particle>),
    Choice(Vec<Particle>),
    All(Vec<Particle>),
}

#[derive(Clone, Debug)]
struct Particle {
    min: u32,
    max: Option<u32>,
    term: Term,
}

#[derive(Clone, Debug)]
enum Content {
    Empty,
    Simple(TypeId),
    Model(Particle),
}

#[derive(Clone, Debug)]
struct ComplexType {
    attrs: Vec<AttrUse>,
    wildcard: Option<Wildcard>,
    content: Content,
    mixed: bool,
}

#[derive(Clone, Debug)]
enum TypeDef {
    Simple(SimpleType),
    Complex(ComplexType),
    /// Being built (recursive reference).
    Pending,
}

#[derive(Clone, Debug)]
struct ElemDecl {
    name: QName,
    ty: TypeId,
    abstract_: bool,
    subst: Vec<ElemId>,
}

struct Schema {
    types: Vec<TypeDef>,
    elems: Vec<ElemDecl>,
    globals: HashMap<QName, ElemId>,
    any_type: TypeId,
    builtin_ids: HashMap<Vec<u8>, TypeId>,
    // compiled content models, per complex type
    nfas: HashMap<TypeId, Nfa>,
}

// ---------------------------------------------------------------- loading

struct Loader<'a> {
    doc: &'a Doc,
    tns: Option<Vec<u8>>,
    efq: bool,
    afq: bool,
    s: Schema,
    named_types: HashMap<QName, NodeId>,
    named_elems: HashMap<QName, NodeId>,
    named_groups: HashMap<QName, NodeId>,
    named_attr_groups: HashMap<QName, NodeId>,
    named_attrs: HashMap<QName, NodeId>,
    type_memo: HashMap<QName, TypeId>,
    errors: Vec<XmlError>,
}

fn xs_children(doc: &Doc, n: NodeId) -> Vec<NodeId> {
    doc.children(n)
        .into_iter()
        .filter(|&c| {
            doc.nodes[c].kind == Kind::Element && doc.nodes[c].ns.as_deref() == Some(XSD_NS)
        })
        .collect()
}

fn attr(doc: &Doc, n: NodeId, name: &[u8]) -> Option<Vec<u8>> {
    doc.nodes[n]
        .attrs
        .iter()
        .find(|&&a| doc.nodes[a].ns.is_none() && doc.nodes[a].name == name)
        .map(|&a| doc.nodes[a].content.clone())
}

fn occurs(doc: &Doc, n: NodeId) -> (u32, Option<u32>) {
    let min = attr(doc, n, b"minOccurs")
        .and_then(|v| String::from_utf8_lossy(&v).trim().parse().ok())
        .unwrap_or(1);
    let max = match attr(doc, n, b"maxOccurs") {
        Some(v) if v == b"unbounded" => None,
        Some(v) => Some(String::from_utf8_lossy(&v).trim().parse().unwrap_or(1)),
        None => Some(1),
    };
    (min, max)
}

impl Loader<'_> {
    fn local(&self, n: NodeId) -> &[u8] {
        &self.doc.nodes[n].name
    }

    /// Resolve a QName-valued attribute in the scope of `n`.
    fn qname(&self, n: NodeId, v: &[u8]) -> QName {
        let v = String::from_utf8_lossy(v).trim().as_bytes().to_vec();
        match v.iter().position(|&c| c == b':') {
            Some(i) => {
                let p = &v[..i];
                (self.doc.lookup_ns(n, p), v[i + 1..].to_vec())
            }
            None => (self.doc.lookup_ns(n, b"").filter(|u| !u.is_empty()), v),
        }
    }

    fn add_type(&mut self, t: TypeDef) -> TypeId {
        self.s.types.push(t);
        self.s.types.len() - 1
    }

    fn schema_err(&mut self, msg: String) {
        self.errors.push(XmlError {
            domain: 16,
            level: 2,
            code: 3004,
            line: 0,
            column: 0,
            message: msg,
            file: None,
            context: None,
        });
    }

    fn type_ref(&mut self, at: NodeId, v: &[u8], owner: &str) -> TypeId {
        let q = self.qname(at, v);
        if q.0.as_deref() == Some(XSD_NS) {
            if let Some(&id) = self.s.builtin_ids.get(&q.1) {
                return id;
            }
        }
        if let Some(&id) = self.type_memo.get(&q) {
            return id;
        }
        let Some(&node) = self.named_types.get(&q) else {
            self.schema_err(format!(
                "{owner}, attribute 'type': The QName value '{}' does not resolve to a(n) type definition.\n",
                String::from_utf8_lossy(v).trim()
            ));
            return self.s.any_type;
        };
        let id = self.add_type(TypeDef::Pending);
        self.type_memo.insert(q.clone(), id);
        let t = if self.local(node) == b"simpleType" {
            TypeDef::Simple(self.simple_type(node, Some(q)))
        } else {
            TypeDef::Complex(self.complex_type(node))
        };
        self.s.types[id] = t;
        id
    }

    fn builtin_of(&self, t: TypeId) -> Builtin {
        match &self.s.types[t] {
            TypeDef::Simple(s) => s.builtin,
            _ => Builtin::AnyType,
        }
    }

    fn simple_type(&mut self, n: NodeId, name: Option<QName>) -> SimpleType {
        let mut st = SimpleType {
            name,
            builtin: Builtin::AnySimpleType,
            variety: Variety::Atomic,
            base: None,
            enums: Vec::new(),
            min_len: None,
            max_len: None,
            len: None,
        };
        for c in xs_children(self.doc, n) {
            match self.local(c) {
                b"restriction" => {
                    let base = match attr(self.doc, c, b"base") {
                        Some(b) => self.type_ref(c, &b, "restriction"),
                        None => {
                            // inline base simpleType
                            let inner = xs_children(self.doc, c)
                                .into_iter()
                                .find(|&x| self.local(x) == b"simpleType");
                            match inner {
                                Some(i) => {
                                    let t = self.simple_type(i, None);
                                    self.add_type(TypeDef::Simple(t))
                                }
                                None => self.s.builtin_ids[&b"anySimpleType".to_vec()],
                            }
                        }
                    };
                    st.base = Some(base);
                    st.builtin = self.builtin_of(base);
                    if let TypeDef::Simple(b) = &self.s.types[base] {
                        st.variety = b.variety.clone();
                    }
                    for f in xs_children(self.doc, c) {
                        let v = attr(self.doc, f, b"value").unwrap_or_default();
                        let num = || String::from_utf8_lossy(&v).trim().parse::<usize>().ok();
                        match self.local(f) {
                            b"enumeration" => st.enums.push(v.clone()),
                            b"minLength" => st.min_len = num(),
                            b"maxLength" => st.max_len = num(),
                            b"length" => st.len = num(),
                            _ => {}
                        }
                    }
                }
                b"list" => {
                    let item = match attr(self.doc, c, b"itemType") {
                        Some(b) => self.type_ref(c, &b, "list"),
                        None => match xs_children(self.doc, c)
                            .into_iter()
                            .find(|&x| self.local(x) == b"simpleType")
                        {
                            Some(i) => {
                                let t = self.simple_type(i, None);
                                self.add_type(TypeDef::Simple(t))
                            }
                            None => self.s.builtin_ids[&b"anySimpleType".to_vec()],
                        },
                    };
                    st.variety = Variety::List(item);
                }
                b"union" => {
                    let mut members = Vec::new();
                    if let Some(m) = attr(self.doc, c, b"memberTypes") {
                        for part in String::from_utf8_lossy(&m).split_whitespace() {
                            members.push(self.type_ref(c, part.as_bytes(), "union"));
                        }
                    }
                    for i in xs_children(self.doc, c) {
                        if self.local(i) == b"simpleType" {
                            let t = self.simple_type(i, None);
                            members.push(self.add_type(TypeDef::Simple(t)));
                        }
                    }
                    st.variety = Variety::Union(members);
                }
                _ => {}
            }
        }
        st
    }

    fn attr_decl(&mut self, n: NodeId, out: &mut Vec<AttrUse>, wildcard: &mut Option<Wildcard>) {
        match self.local(n) {
            b"attribute" => {
                let use_ = attr(self.doc, n, b"use").unwrap_or_default();
                let (name, decl_node) = if let Some(r) = attr(self.doc, n, b"ref") {
                    let q = self.qname(n, &r);
                    let node = self.named_attrs.get(&q).copied();
                    (q, node)
                } else {
                    let local = attr(self.doc, n, b"name").unwrap_or_default();
                    let qualified = match attr(self.doc, n, b"form") {
                        Some(f) => f == b"qualified",
                        None => self.afq,
                    };
                    let ns = if qualified { self.tns.clone() } else { None };
                    ((ns, local), Some(n))
                };
                let ty = match decl_node {
                    Some(d) => match attr(self.doc, d, b"type") {
                        Some(t) => self.type_ref(d, &t, "attribute"),
                        None => match xs_children(self.doc, d)
                            .into_iter()
                            .find(|&x| self.local(x) == b"simpleType")
                        {
                            Some(i) => {
                                let t = self.simple_type(i, None);
                                self.add_type(TypeDef::Simple(t))
                            }
                            None => self.s.builtin_ids[&b"anySimpleType".to_vec()],
                        },
                    },
                    None => self.s.builtin_ids[&b"anySimpleType".to_vec()],
                };
                out.retain(|u| u.name != name);
                out.push(AttrUse {
                    name,
                    ty,
                    required: use_ == b"required",
                    prohibited: use_ == b"prohibited",
                });
            }
            b"attributeGroup" => {
                if let Some(r) = attr(self.doc, n, b"ref") {
                    let q = self.qname(n, &r);
                    if let Some(&g) = self.named_attr_groups.get(&q) {
                        for c in xs_children(self.doc, g) {
                            self.attr_decl(c, out, wildcard);
                        }
                    }
                }
            }
            b"anyAttribute" => {
                *wildcard = Some(self.wildcard(n));
            }
            _ => {}
        }
    }

    fn wildcard(&self, n: NodeId) -> Wildcard {
        let process = match attr(self.doc, n, b"processContents").as_deref() {
            Some(b"skip") => Process::Skip,
            Some(b"lax") => Process::Lax,
            _ => Process::Strict,
        };
        let ns = match attr(self.doc, n, b"namespace") {
            None => WNs::Any,
            Some(v) if v == b"##any" => WNs::Any,
            Some(v) if v == b"##other" => WNs::Other(self.tns.clone()),
            Some(v) => WNs::List(
                String::from_utf8_lossy(&v)
                    .split_whitespace()
                    .map(|t| match t {
                        "##targetNamespace" => self.tns.clone(),
                        "##local" => None,
                        u => Some(u.as_bytes().to_vec()),
                    })
                    .collect(),
            ),
        };
        Wildcard { ns, process }
    }

    fn complex_type(&mut self, n: NodeId) -> ComplexType {
        let mut ct = ComplexType {
            attrs: Vec::new(),
            wildcard: None,
            content: Content::Empty,
            mixed: attr(self.doc, n, b"mixed").as_deref() == Some(b"true"),
        };
        for c in xs_children(self.doc, n) {
            match self.local(c) {
                b"sequence" | b"choice" | b"all" | b"group" => {
                    if let Some(p) = self.particle(c) {
                        ct.content = Content::Model(p);
                    }
                }
                b"attribute" | b"attributeGroup" | b"anyAttribute" => {
                    let mut wc = ct.wildcard.take();
                    self.attr_decl(c, &mut ct.attrs, &mut wc);
                    ct.wildcard = wc;
                }
                b"simpleContent" => {
                    for d in xs_children(self.doc, c) {
                        if matches!(self.local(d), b"extension" | b"restriction") {
                            let base = attr(self.doc, d, b"base")
                                .map(|b| self.type_ref(d, &b, "extension"))
                                .unwrap_or(self.s.any_type);
                            let simple = match &self.s.types[base] {
                                TypeDef::Simple(_) => base,
                                TypeDef::Complex(b) => {
                                    ct.attrs = b.attrs.clone();
                                    ct.wildcard = b.wildcard.clone();
                                    match b.content {
                                        Content::Simple(s) => s,
                                        _ => self.s.builtin_ids[&b"string".to_vec()],
                                    }
                                }
                                TypeDef::Pending => self.s.builtin_ids[&b"string".to_vec()],
                            };
                            let simple = if self.local(d) == b"restriction" {
                                let mut st = self.simple_type(d, None);
                                st.base = Some(simple);
                                st.builtin = self.builtin_of(simple);
                                self.add_type(TypeDef::Simple(st))
                            } else {
                                simple
                            };
                            ct.content = Content::Simple(simple);
                            for e in xs_children(self.doc, d) {
                                let mut wc = ct.wildcard.take();
                                self.attr_decl(e, &mut ct.attrs, &mut wc);
                                ct.wildcard = wc;
                            }
                        }
                    }
                }
                b"complexContent" => {
                    if attr(self.doc, c, b"mixed").as_deref() == Some(b"true") {
                        ct.mixed = true;
                    }
                    for d in xs_children(self.doc, c) {
                        let is_ext = self.local(d) == b"extension";
                        if !is_ext && self.local(d) != b"restriction" {
                            continue;
                        }
                        let base = attr(self.doc, d, b"base")
                            .map(|b| self.type_ref(d, &b, "extension"))
                            .unwrap_or(self.s.any_type);
                        let mut base_model = None;
                        if is_ext {
                            if let TypeDef::Complex(b) = &self.s.types[base] {
                                ct.attrs = b.attrs.clone();
                                ct.wildcard = b.wildcard.clone();
                                if let Content::Model(p) = &b.content {
                                    base_model = Some(p.clone());
                                }
                            }
                        }
                        let mut own = None;
                        for e in xs_children(self.doc, d) {
                            match self.local(e) {
                                b"sequence" | b"choice" | b"all" | b"group" => {
                                    own = self.particle(e)
                                }
                                _ => {
                                    let mut wc = ct.wildcard.take();
                                    self.attr_decl(e, &mut ct.attrs, &mut wc);
                                    ct.wildcard = wc;
                                }
                            }
                        }
                        ct.content = match (base_model, own) {
                            (Some(b), Some(o)) => Content::Model(Particle {
                                min: 1,
                                max: Some(1),
                                term: Term::Seq(vec![b, o]),
                            }),
                            (Some(b), None) => Content::Model(b),
                            (None, Some(o)) => Content::Model(o),
                            (None, None) => Content::Empty,
                        };
                    }
                }
                _ => {}
            }
        }
        ct
    }

    fn particle(&mut self, n: NodeId) -> Option<Particle> {
        let (min, max) = occurs(self.doc, n);
        let term = match self.local(n) {
            b"element" => Term::Elem(self.local_element(n)),
            b"any" => Term::Any(self.wildcard(n)),
            b"sequence" | b"choice" | b"all" => {
                let mut kids = Vec::new();
                for c in xs_children(self.doc, n) {
                    if let Some(p) = self.particle(c) {
                        kids.push(p);
                    }
                }
                match self.local(n) {
                    b"sequence" => Term::Seq(kids),
                    b"choice" => Term::Choice(kids),
                    _ => Term::All(kids),
                }
            }
            b"group" => {
                let r = attr(self.doc, n, b"ref")?;
                let q = self.qname(n, &r);
                let g = *self.named_groups.get(&q)?;
                let inner = xs_children(self.doc, g)
                    .into_iter()
                    .find(|&c| matches!(self.local(c), b"sequence" | b"choice" | b"all"))?;
                let mut p = self.particle(inner)?;
                if (min, max) != (1, Some(1)) {
                    p = Particle {
                        min,
                        max,
                        term: Term::Seq(vec![p]),
                    };
                }
                return Some(p);
            }
            _ => return None,
        };
        Some(Particle { min, max, term })
    }

    fn local_element(&mut self, n: NodeId) -> ElemId {
        if let Some(r) = attr(self.doc, n, b"ref") {
            let q = self.qname(n, &r);
            return self.global_element(&q).unwrap_or(0);
        }
        let local = attr(self.doc, n, b"name").unwrap_or_default();
        let qualified = match attr(self.doc, n, b"form") {
            Some(f) => f == b"qualified",
            None => self.efq,
        };
        let ns = if qualified { self.tns.clone() } else { None };
        let id = self.s.elems.len();
        self.s.elems.push(ElemDecl {
            name: (ns, local),
            ty: self.s.any_type,
            abstract_: false,
            subst: Vec::new(),
        });
        let ty = self.elem_type(n);
        self.s.elems[id].ty = ty;
        id
    }

    fn elem_type(&mut self, n: NodeId) -> TypeId {
        let owner = format!(
            "element decl. '{}'",
            String::from_utf8_lossy(&attr(self.doc, n, b"name").unwrap_or_default())
        );
        if let Some(t) = attr(self.doc, n, b"type") {
            return self.type_ref(n, &t, &owner);
        }
        for c in xs_children(self.doc, n) {
            match self.local(c) {
                b"complexType" => {
                    let id = self.add_type(TypeDef::Pending);
                    let ct = self.complex_type(c);
                    self.s.types[id] = TypeDef::Complex(ct);
                    return id;
                }
                b"simpleType" => {
                    let st = self.simple_type(c, None);
                    return self.add_type(TypeDef::Simple(st));
                }
                _ => {}
            }
        }
        if let Some(sg) = attr(self.doc, n, b"substitutionGroup") {
            let q = self.qname(n, &sg);
            if let Some(h) = self.global_element(&q) {
                return self.s.elems[h].ty;
            }
        }
        self.s.any_type
    }

    fn global_element(&mut self, q: &QName) -> Option<ElemId> {
        if let Some(&id) = self.s.globals.get(q) {
            return Some(id);
        }
        let node = *self.named_elems.get(q)?;
        let id = self.s.elems.len();
        self.s.elems.push(ElemDecl {
            name: q.clone(),
            ty: self.s.any_type,
            abstract_: attr(self.doc, node, b"abstract").as_deref() == Some(b"true"),
            subst: Vec::new(),
        });
        self.s.globals.insert(q.clone(), id);
        let ty = self.elem_type(node);
        self.s.elems[id].ty = ty;
        Some(id)
    }
}

fn new_schema() -> Schema {
    let mut s = Schema {
        types: Vec::new(),
        elems: Vec::new(),
        globals: HashMap::new(),
        any_type: 0,
        builtin_ids: HashMap::new(),
        nfas: HashMap::new(),
    };
    for (name, b) in BUILTINS {
        let id = s.types.len();
        if *b == Builtin::AnyType {
            s.types.push(TypeDef::Complex(ComplexType {
                attrs: Vec::new(),
                wildcard: Some(Wildcard {
                    ns: WNs::Any,
                    process: Process::Lax,
                }),
                content: Content::Model(Particle {
                    min: 0,
                    max: None,
                    term: Term::Any(Wildcard {
                        ns: WNs::Any,
                        process: Process::Lax,
                    }),
                }),
                mixed: true,
            }));
            s.any_type = id;
        } else {
            let variety = match b {
                Builtin::NmTokens | Builtin::IdRefs => Variety::List(0),
                _ => Variety::Atomic,
            };
            s.types.push(TypeDef::Simple(SimpleType {
                name: Some((Some(XSD_NS.to_vec()), name.as_bytes().to_vec())),
                builtin: *b,
                variety,
                base: None,
                enums: Vec::new(),
                min_len: None,
                max_len: None,
                len: None,
            }));
        }
        s.builtin_ids.insert(name.as_bytes().to_vec(), id);
    }
    // NMTOKENS / IDREFS item types
    let nmtoken = s.builtin_ids[&b"NMTOKEN".to_vec()];
    let idref = s.builtin_ids[&b"IDREF".to_vec()];
    for (n, item) in [
        (b"NMTOKENS".to_vec(), nmtoken),
        (b"IDREFS".to_vec(), idref),
        (b"ENTITIES".to_vec(), idref),
    ] {
        let id = s.builtin_ids[&n];
        if let TypeDef::Simple(st) = &mut s.types[id] {
            st.variety = Variety::List(item);
        }
    }
    s
}

fn load_schema(data: &[u8], base: Option<&[u8]>) -> Result<Schema, Vec<XmlError>> {
    let opts = ParseOpts {
        keep_blanks: true,
        ..Default::default()
    };
    let (doc, errs) = parser::parse(data, &opts, base);
    let Some(doc) = doc else {
        return Err(errs);
    };
    let root = doc.root();
    let is_schema = root.is_some_and(|r| {
        doc.nodes[r].name == b"schema" && doc.nodes[r].ns.as_deref() == Some(XSD_NS)
    });
    if !is_schema {
        let name = base
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_else(|| "in_memory_buffer".into());
        return Err(vec![XmlError {
            domain: 16,
            level: 2,
            code: 1772,
            line: 0,
            column: 0,
            message: format!("The XML document '{name}' is not a schema document.\n"),
            file: None,
            context: None,
        }]);
    }
    let root = root.unwrap();
    let mut l = Loader {
        doc: &doc,
        tns: attr(&doc, root, b"targetNamespace").filter(|t| !t.is_empty()),
        efq: attr(&doc, root, b"elementFormDefault").as_deref() == Some(b"qualified"),
        afq: attr(&doc, root, b"attributeFormDefault").as_deref() == Some(b"qualified"),
        s: new_schema(),
        named_types: HashMap::new(),
        named_elems: HashMap::new(),
        named_groups: HashMap::new(),
        named_attr_groups: HashMap::new(),
        named_attrs: HashMap::new(),
        type_memo: HashMap::new(),
        errors: Vec::new(),
    };
    for c in xs_children(&doc, root) {
        let Some(name) = attr(&doc, c, b"name") else {
            continue;
        };
        let q = (l.tns.clone(), name);
        match &doc.nodes[c].name[..] {
            b"simpleType" | b"complexType" => {
                l.named_types.insert(q, c);
            }
            b"element" => {
                l.named_elems.insert(q, c);
            }
            b"group" => {
                l.named_groups.insert(q, c);
            }
            b"attributeGroup" => {
                l.named_attr_groups.insert(q, c);
            }
            b"attribute" => {
                l.named_attrs.insert(q, c);
            }
            _ => {}
        }
    }
    // build every global element (in document order), then wire up
    // substitution groups
    let mut order: Vec<(QName, NodeId)> =
        l.named_elems.iter().map(|(q, &n)| (q.clone(), n)).collect();
    order.sort_by_key(|(_, n)| *n);
    for (q, _) in &order {
        l.global_element(q);
    }
    for (q, n) in &order {
        if let Some(sg) = attr(&doc, *n, b"substitutionGroup") {
            let head = l.qname(*n, &sg);
            if let (Some(&h), Some(&m)) = (l.s.globals.get(&head), l.s.globals.get(q)) {
                l.s.elems[h].subst.push(m);
            }
        }
    }
    if !l.errors.is_empty() {
        return Err(l.errors);
    }
    Ok(l.s)
}

// ---------------------------------------------------------------- NFA

#[derive(Clone, Debug)]
enum Label {
    Elem(ElemId),
    Any(Wildcard),
}

#[derive(Default, Clone, Debug)]
struct NState {
    /// (label, target, creation sequence number)
    trans: Vec<(Label, usize, usize)>,
    /// (target, counted): a counted epsilon stands for one of libxml2's
    /// counter transitions, which its expected-value listing can't see past.
    eps: Vec<(usize, bool)>,
}

#[derive(Clone, Debug)]
enum Nfa {
    Thompson {
        states: Vec<NState>,
        start: usize,
        end: usize,
    },
    All(Vec<(Label, bool)>),
}

struct Builder {
    states: Vec<NState>,
    seq: usize,
}

impl Builder {
    fn new_state(&mut self) -> usize {
        self.states.push(NState::default());
        self.states.len() - 1
    }

    fn eps(&mut self, a: usize, b: usize, counted: bool) {
        self.states[a].eps.push((b, counted));
    }

    /// Build `p` from `from`; returns the end state.
    fn particle(&mut self, p: &Particle, from: usize) -> usize {
        let c = counted_here(p);
        let mut cur = from;
        let min = p.min.min(64);
        for _ in 0..min {
            cur = self.term(&p.term, cur, c);
        }
        match p.max {
            None => {
                // loop: cur -eps-> body -> cur
                let loop_start = self.new_state();
                self.eps(cur, loop_start, c);
                let body_end = self.term(&p.term, loop_start, c);
                self.eps(body_end, loop_start, c);
                let out = self.new_state();
                self.eps(loop_start, out, false);
                out
            }
            Some(max) => {
                let extra = max.saturating_sub(p.min).min(64);
                let out = self.new_state();
                for _ in 0..extra {
                    self.eps(cur, out, false);
                    cur = self.term(&p.term, cur, c);
                }
                self.eps(cur, out, false);
                out
            }
        }
    }

    fn term(&mut self, t: &Term, from: usize, counted: bool) -> usize {
        match t {
            Term::Elem(e) => {
                let to = self.new_state();
                self.seq += 1;
                self.states[from]
                    .trans
                    .push((Label::Elem(*e), to, self.seq));
                to
            }
            Term::Any(w) => {
                let to = self.new_state();
                self.seq += 1;
                self.states[from]
                    .trans
                    .push((Label::Any(w.clone()), to, self.seq));
                to
            }
            Term::Seq(ps) => {
                let mut cur = from;
                for p in ps {
                    cur = self.particle(p, cur);
                }
                cur
            }
            Term::Choice(ps) | Term::All(ps) => {
                let out = self.new_state();
                if ps.is_empty() {
                    self.eps(from, out, false);
                }
                for p in ps {
                    let s = self.new_state();
                    self.eps(from, s, counted);
                    let e = self.particle(p, s);
                    self.eps(e, out, counted);
                }
                out
            }
        }
    }
}

fn compile(p: &Particle) -> Nfa {
    if let Term::All(ps) = &p.term {
        let mut items = Vec::new();
        for q in ps {
            if let Term::Elem(e) = q.term {
                items.push((Label::Elem(e), q.min > 0 && p.min > 0));
            }
        }
        return Nfa::All(items);
    }
    let mut b = Builder {
        states: Vec::new(),
        seq: 0,
    };
    let start = b.new_state();
    let end = b.particle(p, start);
    Nfa::Thompson {
        states: b.states,
        start,
        end,
    }
}

/// Whether `p` itself (not its children) gets a libxml2 counter.
fn counted_here(p: &Particle) -> bool {
    let unbounded = p.max.is_none();
    match &p.term {
        Term::Choice(_) => p.max != Some(1),
        Term::All(_) => true,
        Term::Seq(_) => !unbounded && (p.max.unwrap_or(1) > 1 || p.min > 1),
        Term::Elem(_) | Term::Any(_) => {
            if unbounded {
                p.min >= 2
            } else {
                p.max.unwrap_or(1) > 1 || p.min > 1
            }
        }
    }
}

fn closure(states: &[NState], set: &mut Vec<usize>, follow_counted: bool) {
    let mut i = 0;
    while i < set.len() {
        let st = set[i];
        for &(e, counted) in &states[st].eps {
            if (follow_counted || !counted) && !set.contains(&e) {
                set.push(e);
            }
        }
        i += 1;
    }
    set.sort_unstable();
}

// ---------------------------------------------------------------- validation

struct V<'a> {
    s: &'a mut Schema,
    doc: &'a Doc,
    errors: Vec<XmlError>,
}

fn fmt_qname(ns: &Option<Vec<u8>>, local: &[u8]) -> String {
    match ns {
        Some(u) if !u.is_empty() => format!(
            "{{{}}}{}",
            String::from_utf8_lossy(u),
            String::from_utf8_lossy(local)
        ),
        _ => String::from_utf8_lossy(local).into_owned(),
    }
}

fn collapse(s: &[u8]) -> Vec<u8> {
    let st = String::from_utf8_lossy(s);
    st.split([' ', '\t', '\n', '\r'])
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .into_bytes()
}

fn replace_ws(s: &[u8]) -> Vec<u8> {
    s.iter()
        .map(|&c| {
            if matches!(c, b'\t' | b'\n' | b'\r') {
                b' '
            } else {
                c
            }
        })
        .collect()
}

fn is_ncname(s: &str) -> bool {
    let mut it = s.chars();
    match it.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    it.all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '\u{b7}'))
}

fn is_name(s: &str) -> bool {
    let mut it = s.chars();
    match it.next() {
        Some(c) if c.is_alphabetic() || c == '_' || c == ':' => {}
        _ => return false,
    }
    it.all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | '\u{b7}'))
}

fn int_value(s: &str) -> Option<i128> {
    let b = s.as_bytes();
    let (neg, digits) = match b.first() {
        Some(b'+') => (false, &s[1..]),
        Some(b'-') => (true, &s[1..]),
        _ => (false, s),
    };
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let t = digits.trim_start_matches('0');
    if t.len() > 36 {
        return Some(if neg { i128::MIN } else { i128::MAX });
    }
    let v: i128 = if t.is_empty() { 0 } else { t.parse().ok()? };
    Some(if neg { -v } else { v })
}

fn is_decimal(s: &str) -> bool {
    let t = s.strip_prefix(['+', '-']).unwrap_or(s);
    if t.is_empty() || t == "." {
        return false;
    }
    let mut dot = false;
    for c in t.chars() {
        if c == '.' {
            if dot {
                return false;
            }
            dot = true;
        } else if !c.is_ascii_digit() {
            return false;
        }
    }
    true
}

fn is_float(s: &str) -> bool {
    if matches!(s, "INF" | "-INF" | "NaN" | "+INF") {
        return s != "+INF";
    }
    let (m, e) = match s.find(['e', 'E']) {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    if !is_decimal(m) {
        return false;
    }
    match e {
        None => true,
        Some(e) => int_value(e).is_some(),
    }
}

fn digits_n(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|c| c.is_ascii_digit())
}

fn tz_ok(s: &str) -> bool {
    s.is_empty() || s == "Z" || {
        let b = s.as_bytes();
        (b[0] == b'+' || b[0] == b'-')
            && s.len() == 6
            && digits_n(&s[1..3], 2)
            && &s[3..4] == ":"
            && digits_n(&s[4..6], 2)
    }
}

fn date_ok(s: &str) -> Option<usize> {
    // [-]YYYY-MM-DD, returns consumed length
    let off = if s.starts_with('-') { 1 } else { 0 };
    let r = &s[off..];
    let ydig = r.bytes().take_while(|c| c.is_ascii_digit()).count();
    if ydig < 4 || r.len() < ydig + 6 {
        return None;
    }
    let rest = &r[ydig..];
    if &rest[0..1] != "-"
        || !digits_n(&rest[1..3], 2)
        || &rest[3..4] != "-"
        || !digits_n(&rest[4..6], 2)
    {
        return None;
    }
    let (m, d): (u32, u32) = (rest[1..3].parse().ok()?, rest[4..6].parse().ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(off + ydig + 6)
}

fn time_ok(s: &str) -> Option<usize> {
    if s.len() < 8
        || !digits_n(&s[0..2], 2)
        || &s[2..3] != ":"
        || !digits_n(&s[3..5], 2)
        || &s[5..6] != ":"
        || !digits_n(&s[6..8], 2)
    {
        return None;
    }
    let mut n = 8;
    if s[n..].starts_with('.') {
        let f = s[n + 1..]
            .bytes()
            .take_while(|c| c.is_ascii_digit())
            .count();
        if f == 0 {
            return None;
        }
        n += 1 + f;
    }
    Some(n)
}

fn builtin_valid(b: Builtin, v: &[u8]) -> bool {
    let s = String::from_utf8_lossy(v);
    let s = s.as_ref();
    match b {
        Builtin::AnyType
        | Builtin::AnySimpleType
        | Builtin::String
        | Builtin::NormalizedString
        | Builtin::Token => true,
        Builtin::AnyUri => true,
        Builtin::Boolean => matches!(s, "true" | "false" | "1" | "0"),
        Builtin::Decimal => is_decimal(s),
        Builtin::Float | Builtin::Double => is_float(s),
        Builtin::Integer => int_value(s).is_some(),
        Builtin::NonPositiveInteger => int_value(s).is_some_and(|v| v <= 0),
        Builtin::NegativeInteger => int_value(s).is_some_and(|v| v < 0),
        Builtin::NonNegativeInteger => int_value(s).is_some_and(|v| v >= 0),
        Builtin::PositiveInteger => int_value(s).is_some_and(|v| v > 0),
        Builtin::Long => {
            int_value(s).is_some_and(|v| v >= i64::MIN as i128 && v <= i64::MAX as i128)
        }
        Builtin::Int => {
            int_value(s).is_some_and(|v| v >= i32::MIN as i128 && v <= i32::MAX as i128)
        }
        Builtin::Short => {
            int_value(s).is_some_and(|v| v >= i16::MIN as i128 && v <= i16::MAX as i128)
        }
        Builtin::Byte => int_value(s).is_some_and(|v| v >= i8::MIN as i128 && v <= i8::MAX as i128),
        Builtin::UnsignedLong => int_value(s).is_some_and(|v| v >= 0 && v <= u64::MAX as i128),
        Builtin::UnsignedInt => int_value(s).is_some_and(|v| v >= 0 && v <= u32::MAX as i128),
        Builtin::UnsignedShort => int_value(s).is_some_and(|v| v >= 0 && v <= u16::MAX as i128),
        Builtin::UnsignedByte => int_value(s).is_some_and(|v| v >= 0 && v <= u8::MAX as i128),
        Builtin::Name => is_name(s),
        Builtin::NCName | Builtin::Id | Builtin::IdRef | Builtin::Entity => is_ncname(s),
        Builtin::QName | Builtin::Notation => match s.split_once(':') {
            Some((p, l)) => is_ncname(p) && is_ncname(l),
            None => is_ncname(s),
        },
        Builtin::NmToken => {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | '\u{b7}'))
        }
        Builtin::NmTokens | Builtin::IdRefs => !s.is_empty(),
        Builtin::Language => {
            let parts: Vec<&str> = s.split('-').collect();
            !parts.is_empty()
                && parts[0].len() <= 8
                && !parts[0].is_empty()
                && parts[0].chars().all(|c| c.is_ascii_alphabetic())
                && parts[1..].iter().all(|p| {
                    !p.is_empty() && p.len() <= 8 && p.chars().all(|c| c.is_ascii_alphanumeric())
                })
        }
        Builtin::Date => date_ok(s).is_some_and(|n| tz_ok(&s[n..])),
        Builtin::Time => time_ok(s).is_some_and(|n| tz_ok(&s[n..])),
        Builtin::DateTime => date_ok(s).is_some_and(|n| {
            s[n..].starts_with('T') && time_ok(&s[n + 1..]).is_some_and(|m| tz_ok(&s[n + 1 + m..]))
        }),
        Builtin::Duration => {
            let t = s.strip_prefix('-').unwrap_or(s);
            t.starts_with('P')
                && t.len() > 1
                && !t.ends_with('T')
                && t[1..]
                    .chars()
                    .all(|c| c.is_ascii_digit() || "YMDTHS.".contains(c))
        }
        Builtin::GYear => {
            let t = s.strip_prefix('-').unwrap_or(s);
            let n = t.bytes().take_while(|c| c.is_ascii_digit()).count();
            n >= 4 && tz_ok(&t[n..])
        }
        Builtin::GYearMonth | Builtin::GMonth | Builtin::GMonthDay | Builtin::GDay => !s.is_empty(),
        Builtin::Base64Binary => s
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'=' | b' ')),
        Builtin::HexBinary => s.len() % 2 == 0 && s.bytes().all(|c| c.is_ascii_hexdigit()),
    }
}

fn whitespace_of(b: Builtin) -> u8 {
    match b {
        Builtin::String | Builtin::AnySimpleType | Builtin::AnyType => 0,
        Builtin::NormalizedString => 1,
        _ => 2,
    }
}

/// Outcome of simple-type validation: Ok, or (code, message tail).
type SResult = Result<(), (i32, String)>;

impl V<'_> {
    fn err(&mut self, node: NodeId, code: i32, msg: String) {
        self.errors.push(XmlError {
            domain: 17,
            level: 2,
            code,
            line: self.doc.nodes[node].line as i32,
            column: 0,
            message: msg,
            file: self.doc.url.clone(),
            context: None,
        });
    }

    fn node_desc(&self, n: NodeId) -> String {
        let node = &self.doc.nodes[n];
        if node.kind == Kind::Attr {
            let el = node.parent.unwrap_or(0);
            let e = &self.doc.nodes[el];
            format!(
                "Element '{}', attribute '{}'",
                fmt_qname(&e.ns, &e.name),
                fmt_qname(&node.ns, &node.name)
            )
        } else {
            format!("Element '{}'", fmt_qname(&node.ns, &node.name))
        }
    }

    fn type_desc(&self, t: TypeId) -> String {
        let TypeDef::Simple(st) = &self.s.types[t] else {
            return "complex type".into();
        };
        let kind = match st.variety {
            Variety::Atomic => "atomic type",
            Variety::List(_) => "list type",
            Variety::Union(_) => "union type",
        };
        match &st.name {
            Some((Some(ns), local)) if ns == XSD_NS => {
                format!("{kind} 'xs:{}'", String::from_utf8_lossy(local))
            }
            Some((ns, local)) => format!("{kind} '{}'", fmt_qname(ns, local)),
            None => format!("local {kind}"),
        }
    }

    /// Validate `value` against simple type `t`.
    fn simple(&self, t: TypeId, value: &[u8]) -> SResult {
        let TypeDef::Simple(st) = &self.s.types[t] else {
            return Ok(());
        };
        let st = st.clone();
        let ws = whitespace_of(st.builtin);
        let norm = match ws {
            0 => value.to_vec(),
            1 => replace_ws(value),
            _ => collapse(value),
        };
        let fail_type = || -> (i32, String) {
            let code = match st.variety {
                Variety::Union(_) => 1826,
                Variety::List(_) => 1825,
                Variety::Atomic => 1824,
            };
            (
                code,
                format!(
                    "'{}' is not a valid value of the {}.\n",
                    String::from_utf8_lossy(&norm),
                    self.type_desc(t)
                ),
            )
        };
        match &st.variety {
            Variety::Union(members) => {
                if !members.iter().any(|&m| self.simple(m, value).is_ok()) {
                    return Err(fail_type());
                }
            }
            Variety::List(item) => {
                for part in String::from_utf8_lossy(&norm)
                    .split(' ')
                    .filter(|p| !p.is_empty())
                {
                    if self.simple(*item, part.as_bytes()).is_err() {
                        return Err(fail_type());
                    }
                }
            }
            Variety::Atomic => {
                // the base chain first (builtin lexical space)
                if let Some(b) = st.base {
                    if let Err(e) = self.simple(b, value) {
                        // facet errors of the base propagate; lexical errors
                        // are reported against this type
                        if e.0 == 1824 && st.name.is_none() {
                            return Err(fail_type());
                        }
                        if e.0 == 1824 {
                            return Err(fail_type());
                        }
                        return Err(e);
                    }
                } else if !builtin_valid(st.builtin, &norm) {
                    return Err(fail_type());
                }
            }
        }
        let len = String::from_utf8_lossy(&norm).chars().count();
        if let Some(l) = st.len {
            if len != l {
                return Err((
                    1830,
                    format!(
                        "[facet 'length'] The value has a length of '{len}'; this differs from the allowed length of '{l}'.\n"
                    ),
                ));
            }
        }
        if let Some(l) = st.min_len {
            if len < l {
                return Err((
                    1831,
                    format!(
                        "[facet 'minLength'] The value has a length of '{len}'; this underruns the allowed minimum length of '{l}'.\n"
                    ),
                ));
            }
        }
        if let Some(l) = st.max_len {
            if len > l {
                return Err((
                    1832,
                    format!(
                        "[facet 'maxLength'] The value has a length of '{len}'; this exceeds the allowed maximum length of '{l}'.\n"
                    ),
                ));
            }
        }
        if !st.enums.is_empty() {
            let ok = st.enums.iter().any(|e| {
                let en = match ws {
                    0 => e.clone(),
                    1 => replace_ws(e),
                    _ => collapse(e),
                };
                en == norm
            });
            if !ok {
                let set: Vec<String> = st
                    .enums
                    .iter()
                    .map(|e| format!("'{}'", String::from_utf8_lossy(e)))
                    .collect();
                return Err((
                    1840,
                    format!(
                        "[facet 'enumeration'] The value '{}' is not an element of the set {{{}}}.\n",
                        String::from_utf8_lossy(&norm),
                        set.join(", ")
                    ),
                ));
            }
        }
        Ok(())
    }

    fn check_simple(&mut self, node: NodeId, t: TypeId, value: &[u8]) {
        if let Err((code, msg)) = self.simple(t, value) {
            let d = self.node_desc(node);
            self.err(node, code, format!("{d}: {msg}"));
        }
    }

    fn elem_matches(
        &self,
        decl: ElemId,
        ns: &Option<Vec<u8>>,
        local: &[u8],
        depth: usize,
    ) -> Option<ElemId> {
        let e = &self.s.elems[decl];
        if e.name.0 == *ns && e.name.1 == local {
            return Some(decl);
        }
        if depth > 16 {
            return None;
        }
        for &m in &e.subst {
            if let Some(x) = self.elem_matches(m, ns, local, depth + 1) {
                return Some(x);
            }
        }
        None
    }

    /// A label as listed in "Expected is ( ... )": an element declaration
    /// brings its substitution-group members along.
    fn label_names(&self, l: &Label) -> Vec<String> {
        let mut out = vec![self.label_name(l)];
        if let Label::Elem(e) = l {
            let mut stack: Vec<ElemId> = self.s.elems[*e].subst.iter().rev().copied().collect();
            let mut guard = 0;
            while let Some(m) = stack.pop() {
                guard += 1;
                if guard > 256 {
                    break;
                }
                out.push(self.label_name(&Label::Elem(m)));
                stack.extend(self.s.elems[m].subst.iter().rev().copied());
            }
        }
        out
    }

    fn label_name(&self, l: &Label) -> String {
        match l {
            Label::Elem(e) => {
                let d = &self.s.elems[*e];
                fmt_qname(&d.name.0, &d.name.1)
            }
            Label::Any(w) => match &w.ns {
                WNs::Other(t) => format!(
                    "##other{{{}}}*",
                    String::from_utf8_lossy(t.as_deref().unwrap_or(b""))
                ),
                _ => "*".into(),
            },
        }
    }

    fn expected(&self, names: &[String]) -> String {
        let mut uniq: Vec<&String> = Vec::new();
        for n in names {
            if !uniq.contains(&n) {
                uniq.push(n);
            }
        }
        uniq.truncate(10);
        match uniq.len() {
            0 => String::new(),
            1 => format!(" Expected is ( {} ).", uniq[0]),
            _ => format!(
                " Expected is one of ( {} ).",
                uniq.iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    fn validate_root(&mut self, root: NodeId) {
        let n = &self.doc.nodes[root];
        let q = (n.ns.clone(), n.name.clone());
        match self.s.globals.get(&q).copied() {
            Some(e) if !self.s.elems[e].abstract_ => self.element(root, e),
            Some(_) => {
                let d = self.node_desc(root);
                self.err(
                    root,
                    1846,
                    format!("{d}: The element declaration is abstract.\n"),
                );
            }
            None => {
                let d = self.node_desc(root);
                self.err(
                    root,
                    1845,
                    format!(
                        "{d}: No matching global declaration available for the validation root.\n"
                    ),
                );
            }
        }
    }

    fn element(&mut self, el: NodeId, decl: ElemId) {
        let ty = self.s.elems[decl].ty;
        match self.s.types[ty].clone() {
            TypeDef::Simple(_) => {
                for &a in &self.doc.nodes[el].attrs {
                    if self.doc.nodes[a].ns.as_deref() == Some(XSI_NS) {
                        continue;
                    }
                    let d = self.node_desc(a);
                    let an = &self.doc.nodes[a];
                    let shown = fmt_qname(&an.ns, &an.name);
                    self.err(
                        el,
                        1866,
                        format!("{d}: The attribute '{shown}' is not allowed.\n"),
                    );
                }
                if self
                    .doc
                    .children(el)
                    .iter()
                    .any(|&c| self.doc.nodes[c].kind == Kind::Element)
                {
                    let d = self.node_desc(el);
                    self.err(el, 1842, format!("{d}: Element content is not allowed, because the type definition is simple.\n"));
                    return;
                }
                let v = self.doc.text_content(el);
                self.check_simple(el, ty, &v);
            }
            TypeDef::Complex(ct) => self.complex(el, ty, &ct),
            TypeDef::Pending => {}
        }
    }

    fn attributes(&mut self, el: NodeId, ct: &ComplexType) {
        let attrs = self.doc.nodes[el].attrs.clone();
        let mut matched = vec![false; ct.attrs.len()];
        let mut unknown = Vec::new();
        let mut checks: Vec<(NodeId, TypeId)> = Vec::new();
        for &a in &attrs {
            let an = &self.doc.nodes[a];
            if an.ns.as_deref() == Some(XSI_NS) {
                continue;
            }
            let pos = ct
                .attrs
                .iter()
                .position(|u| u.name.0 == an.ns && u.name.1 == an.name && !u.prohibited);
            match pos {
                Some(i) => {
                    matched[i] = true;
                    checks.push((a, ct.attrs[i].ty));
                }
                None => {
                    let ok = ct.wildcard.as_ref().is_some_and(|w| w.allows(&an.ns));
                    if !ok {
                        unknown.push(a);
                    }
                }
            }
        }
        for (a, t) in checks {
            let v = self.doc.nodes[a].content.clone();
            self.check_simple(a, t, &v);
        }
        // cvc-complex-type 3.2.1 (no wildcard) vs 3.2.2 (wildcard rejects it)
        let code = if ct.wildcard.is_some() { 1867 } else { 1866 };
        for a in unknown {
            let d = self.node_desc(a);
            let an = &self.doc.nodes[a];
            let shown = fmt_qname(&an.ns, &an.name);
            self.err(
                el,
                code,
                format!("{d}: The attribute '{shown}' is not allowed.\n"),
            );
        }
        for (i, u) in ct.attrs.iter().enumerate() {
            if u.required && !matched[i] {
                let d = self.node_desc(el);
                self.err(
                    el,
                    1868,
                    format!(
                        "{d}: The attribute '{}' is required but missing.\n",
                        fmt_qname(&u.name.0, &u.name.1)
                    ),
                );
            }
        }
    }

    fn complex(&mut self, el: NodeId, ty: TypeId, ct: &ComplexType) {
        self.attributes(el, ct);
        let kids = self.doc.children(el);
        match &ct.content {
            Content::Empty => {
                for &c in &kids {
                    match self.doc.nodes[c].kind {
                        Kind::Element => {
                            let d = self.node_desc(el);
                            self.err(el, 1841, format!("{d}: Element content is not allowed, because the content type is empty.\n"));
                            return;
                        }
                        Kind::Text | Kind::CData | Kind::EntityRef => {
                            let t = self.doc.text_content(c);
                            if !ct.mixed
                                && t.iter()
                                    .any(|&b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
                            {
                                let d = self.node_desc(el);
                                self.err(el, 1841, format!("{d}: Character content is not allowed, because the content type is empty.\n"));
                                return;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Content::Simple(st) => {
                if kids
                    .iter()
                    .any(|&c| self.doc.nodes[c].kind == Kind::Element)
                {
                    let d = self.node_desc(el);
                    self.err(el, 1842, format!("{d}: Element content is not allowed, because the content type is a simple type definition.\n"));
                    return;
                }
                let v = self.doc.text_content(el);
                self.check_simple(el, *st, &v);
            }
            Content::Model(p) => {
                self.s.nfas.entry(ty).or_insert_with(|| compile(p));
                let nfa = self.s.nfas[&ty].clone();
                self.model(el, &nfa, ct.mixed, &kids);
            }
        }
    }

    fn match_label(&self, l: &Label, c: NodeId) -> Option<Option<ElemId>> {
        let n = &self.doc.nodes[c];
        match l {
            Label::Elem(e) => self.elem_matches(*e, &n.ns, &n.name, 0).map(Some),
            Label::Any(w) => {
                if !w.allows(&n.ns) {
                    return None;
                }
                Some(match w.process {
                    Process::Skip => None,
                    _ => self.s.globals.get(&(n.ns.clone(), n.name.clone())).copied(),
                })
            }
        }
    }

    fn model(&mut self, el: NodeId, nfa: &Nfa, mixed: bool, kids: &[NodeId]) {
        match nfa {
            Nfa::Thompson { states, start, end } => {
                let mut cur = vec![*start];
                let mut plain = cur.clone();
                closure(states, &mut cur, true);
                closure(states, &mut plain, false);
                for &c in kids {
                    match self.doc.nodes[c].kind {
                        Kind::Element => {}
                        Kind::Text | Kind::CData | Kind::EntityRef => {
                            if !mixed && self.char_content(el, c) {
                                continue;
                            }
                            continue;
                        }
                        _ => continue,
                    }
                    let mut next = Vec::new();
                    let mut decl: Option<Option<ElemId>> = None;
                    for &s in &cur {
                        for (l, to, _) in &states[s].trans {
                            if let Some(d) = self.match_label(l, c) {
                                if decl.is_none() {
                                    decl = Some(d);
                                }
                                if !next.contains(to) {
                                    next.push(*to);
                                }
                            }
                        }
                    }
                    if next.is_empty() {
                        let names = self.state_labels(states, &plain);
                        let d = self.node_desc(c);
                        let exp = self.expected(&names);
                        self.err(
                            c,
                            1871,
                            format!("{d}: This element is not expected.{exp}\n"),
                        );
                        return;
                    }
                    plain = next.clone();
                    closure(states, &mut next, true);
                    closure(states, &mut plain, false);
                    cur = next;
                    if let Some(Some(d)) = decl {
                        if self.s.elems[d].abstract_ {
                            let desc = self.node_desc(c);
                            self.err(
                                c,
                                1846,
                                format!("{desc}: The element declaration is abstract.\n"),
                            );
                        } else {
                            self.element(c, d);
                        }
                    }
                }
                if !cur.contains(end) {
                    let names = self.state_labels(states, &plain);
                    let d = self.node_desc(el);
                    let exp = self.expected(&names);
                    self.err(el, 1871, format!("{d}: Missing child element(s).{exp}\n"));
                }
            }
            Nfa::All(items) => {
                let mut seen = vec![false; items.len()];
                let mut any = false;
                for &c in kids {
                    match self.doc.nodes[c].kind {
                        Kind::Element => {}
                        Kind::Text | Kind::CData | Kind::EntityRef => {
                            if !mixed {
                                self.char_content(el, c);
                            }
                            continue;
                        }
                        _ => continue,
                    }
                    let mut hit = None;
                    for (i, (l, _)) in items.iter().enumerate() {
                        if seen[i] {
                            continue;
                        }
                        if let Some(d) = self.match_label(l, c) {
                            hit = Some((i, d));
                            break;
                        }
                    }
                    match hit {
                        Some((i, d)) => {
                            seen[i] = true;
                            any = true;
                            if let Some(d) = d {
                                self.element(c, d);
                            }
                        }
                        None => {
                            let d = self.node_desc(c);
                            self.err(c, 1871, format!("{d}: This element is not expected.\n"));
                            return;
                        }
                    }
                }
                let missing: Vec<usize> = (0..items.len())
                    .filter(|&i| items[i].1 && !seen[i])
                    .collect();
                if !missing.is_empty() {
                    let names: Vec<String> = if any {
                        missing
                            .iter()
                            .map(|&i| self.label_name(&items[i].0))
                            .collect()
                    } else {
                        items.iter().map(|(l, _)| self.label_name(l)).collect()
                    };
                    let d = self.node_desc(el);
                    let exp = self.expected(&names);
                    self.err(el, 1871, format!("{d}: Missing child element(s).{exp}\n"));
                }
            }
        }
    }

    /// The labels leaving a state set, in schema (creation) order.
    fn state_labels(&self, states: &[NState], cur: &[usize]) -> Vec<String> {
        let mut t: Vec<(usize, String)> = cur
            .iter()
            .flat_map(|&s| {
                states[s]
                    .trans
                    .iter()
                    .flat_map(|(l, _, q)| {
                        self.label_names(l)
                            .into_iter()
                            .map(|n| (*q, n))
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        t.sort_by_key(|x| x.0);
        t.into_iter().map(|x| x.1).collect()
    }

    /// Non-whitespace character data in element-only content: reported.
    fn char_content(&mut self, el: NodeId, c: NodeId) -> bool {
        let t = self.doc.text_content(c);
        if t.iter()
            .any(|&b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        {
            let d = self.node_desc(el);
            self.err(
                el,
                1843,
                format!("{d}: Character content other than whitespace is not allowed because the content type is 'element-only'.\n"),
            );
            return true;
        }
        false
    }
}

/// Parse the schema in `data` and validate `doc` against it: (schema ok,
/// schema-parse errors, validation errors).
pub fn validate_source(
    data: &[u8],
    base: Option<&[u8]>,
    doc: &Doc,
) -> (bool, Vec<XmlError>, Vec<XmlError>) {
    let mut schema = match load_schema(data, base) {
        Ok(s) => s,
        Err(e) => return (false, e, Vec::new()),
    };
    let Some(root) = doc.root() else {
        return (true, Vec::new(), Vec::new());
    };
    let mut v = V {
        s: &mut schema,
        doc,
        errors: Vec::new(),
    };
    v.validate_root(root);
    (true, Vec::new(), v.errors)
}
