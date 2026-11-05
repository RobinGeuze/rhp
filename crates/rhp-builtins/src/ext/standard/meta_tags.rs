//! `get_meta_tags()` from ext/standard/file.c: php's small tokenizer over
//! the file's bytes (`php_next_meta_token`), up to `</head>`.

use rhp_values::ops::deref;
use rhp_values::{conv, ArrayKey, PhpArray, Value};

use crate::{Host, Thrown};

const META_DEF_BUFSIZE: usize = 8192;
const PHP_META_UNSAFE: &[u8] = b".\\+*?[^]$() ";
const PHP_META_HTML401_CHARS: &[u8] = b"-_.:";

#[derive(Clone, Copy, PartialEq)]
enum Tok {
    Eof,
    OpenTag,
    CloseTag,
    Slash,
    Equal,
    Space,
    Id,
    Str,
    Other,
}

/// A read-only plain stream: `getc` returns -1 at the end and only then
/// sets the eof flag, as the plain-files wrapper does.
struct Meta {
    data: Vec<u8>,
    pos: usize,
    eof_flag: bool,
    ulc: bool,
    lc: i32,
    in_meta: bool,
    token: Vec<u8>,
}

impl Meta {
    fn eof(&self) -> bool {
        self.pos >= self.data.len() && self.eof_flag
    }
    fn getc(&mut self) -> i32 {
        match self.data.get(self.pos) {
            Some(&c) => {
                self.pos += 1;
                c as i32
            }
            None => {
                self.eof_flag = true;
                -1
            }
        }
    }

    fn next_token(&mut self) -> Tok {
        let alnum = |c: i32| (0..128).contains(&c) && (c as u8).is_ascii_alphanumeric();
        let mut buf: Vec<u8> = Vec::new();
        let mut ch: i32 = 0;
        loop {
            if !self.ulc {
                if self.eof() {
                    break;
                }
                ch = self.getc();
                if ch == 0 {
                    break;
                }
            }
            if self.eof() {
                break;
            }
            if self.ulc {
                ch = self.lc;
                self.ulc = false;
            }
            match ch {
                0x3c => return Tok::OpenTag,
                0x3e => return Tok::CloseTag,
                0x3d => return Tok::Equal,
                0x2f => return Tok::Slash,
                0x27 | 0x22 => {
                    let compliment = ch;
                    buf.clear();
                    while !self.eof() {
                        ch = self.getc();
                        if ch == 0 || ch == compliment || ch == 0x3c || ch == 0x3e {
                            break;
                        }
                        buf.push(ch as u8);
                        if buf.len() == META_DEF_BUFSIZE {
                            break;
                        }
                    }
                    if ch == 0x3c || ch == 0x3e {
                        // Was just an apostrophe.
                        self.ulc = true;
                        self.lc = ch;
                    }
                    if self.in_meta {
                        self.token = buf;
                    }
                    return Tok::Str;
                }
                0x0a | 0x0d | 0x09 => {}
                0x20 => return Tok::Space,
                _ => {
                    if !alnum(ch) {
                        return Tok::Other;
                    }
                    buf.clear();
                    buf.push(ch as u8);
                    while !self.eof() {
                        ch = self.getc();
                        if ch == 0
                            || !(alnum(ch)
                                || ((0..128).contains(&ch)
                                    && PHP_META_HTML401_CHARS.contains(&(ch as u8))))
                        {
                            break;
                        }
                        buf.push(ch as u8);
                        if buf.len() == META_DEF_BUFSIZE {
                            break;
                        }
                    }
                    let alpha = (0..128).contains(&ch) && (ch as u8).is_ascii_alphabetic();
                    if !alpha && ch != 0x2d {
                        self.ulc = true;
                        self.lc = ch;
                    }
                    self.token = buf;
                    return Tok::Id;
                }
            }
        }
        Tok::Eof
    }
}

fn unsafe_to_underscore(s: &[u8]) -> Vec<u8> {
    // estrndup stops nothing, but the loop stops at a NUL byte.
    let mut out = s.to_vec();
    for c in out.iter_mut() {
        if *c == 0 {
            break;
        }
        if PHP_META_UNSAFE.contains(c) {
            *c = b'_';
        }
    }
    out
}

/// `get_meta_tags(string $filename, bool $use_include_path = false): array|false`.
fn fn_get_meta_tags(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    use crate::ext::standard::file::{include_path_target, path_param};
    let filename = path_param(args, 0, "get_meta_tags", "filename")?;
    let use_include_path = args.get(1).is_some_and(|v| conv::to_bool(&deref(v)));
    let target = if use_include_path {
        include_path_target(host, &filename)
    } else {
        filename.clone()
    };
    let Some(data) = crate::ext::standard::md5::read_file_for_hash(host, "get_meta_tags", &target)?
    else {
        return Ok(Value::Bool(false));
    };
    let mut md = Meta {
        data,
        pos: 0,
        eof_flag: false,
        ulc: false,
        lc: 0,
        in_meta: false,
        token: Vec::new(),
    };
    let mut out = PhpArray::new();
    let (mut in_tag, mut done, mut looking_for_val) = (false, false, false);
    let (mut have_name, mut have_content, mut saw_name, mut saw_content) =
        (false, false, false, false);
    let mut name: Option<Vec<u8>> = None;
    let mut value: Option<Vec<u8>> = None;
    let mut tok_last = Tok::Eof;
    while !done {
        md.token.clear();
        let tok = md.next_token();
        if tok == Tok::Eof {
            break;
        }
        let token = std::mem::take(&mut md.token);
        let eq = |s: &[u8]| token.eq_ignore_ascii_case(s);
        if tok == Tok::Id {
            if tok_last == Tok::OpenTag {
                md.in_meta = eq(b"meta");
            } else if tok_last == Tok::Slash && in_tag {
                if eq(b"head") {
                    done = true;
                }
            } else if tok_last == Tok::Equal && looking_for_val {
                if saw_name {
                    name = Some(unsafe_to_underscore(&token));
                    have_name = true;
                } else if saw_content {
                    value = Some(token.clone());
                    have_content = true;
                }
                looking_for_val = false;
            } else if md.in_meta {
                if eq(b"name") {
                    (saw_name, saw_content, looking_for_val) = (true, false, true);
                } else if eq(b"content") {
                    (saw_name, saw_content, looking_for_val) = (false, true, true);
                }
            }
        } else if tok == Tok::Str && tok_last == Tok::Equal && looking_for_val {
            if saw_name {
                name = Some(unsafe_to_underscore(&token));
                have_name = true;
            } else if saw_content {
                value = Some(token.clone());
                have_content = true;
            }
            looking_for_val = false;
        } else if tok == Tok::OpenTag {
            if looking_for_val {
                looking_for_val = false;
                (have_name, saw_name, have_content, saw_content) = (false, false, false, false);
            }
            in_tag = true;
        } else if tok == Tok::CloseTag {
            if have_name {
                // strlen: the key ends at a NUL byte.
                let mut n = name.take().unwrap_or_default();
                if let Some(z) = n.iter().position(|&c| c == 0) {
                    n.truncate(z);
                }
                n.make_ascii_lowercase();
                let v = if have_content {
                    value.take().unwrap_or_default()
                } else {
                    Vec::new()
                };
                out.insert(ArrayKey::from_bytes(&n), Value::str(v));
            }
            name = None;
            value = None;
            in_tag = false;
            looking_for_val = false;
            (have_name, saw_name, have_content, saw_content) = (false, false, false, false);
            md.in_meta = false;
        }
        tok_last = tok;
    }
    Ok(Value::Array(out))
}

pub fn register(m: &mut crate::Registry) {
    crate::register(m, &[("get_meta_tags", fn_get_meta_tags)]);
}
