//! main/php_glob.c: php's own glob(3), which `glob()` and the glob://
//! wrapper use.

// --- glob (a port of php's bundled OpenBSD glob, main/php_glob.c) ------------

pub(crate) const GLOB_ERR: i64 = 0x0004;
pub(crate) const GLOB_MARK: i64 = 0x0008;
pub(crate) const GLOB_NOCHECK: i64 = 0x0010;
pub(crate) const GLOB_NOSORT: i64 = 0x0020;
pub(crate) const GLOB_BRACE: i64 = 0x0080;
const GLOB_MAGCHAR: i64 = 0x0100;
pub(crate) const GLOB_NOESCAPE: i64 = 0x1000;
pub(crate) const GLOB_ONLYDIR: i64 = 1 << 30;
pub(crate) const GLOB_AVAILABLE: i64 =
    GLOB_BRACE | GLOB_MARK | GLOB_NOSORT | GLOB_NOCHECK | GLOB_NOESCAPE | GLOB_ERR | GLOB_ONLYDIR;
pub(crate) const GLOB_NOMATCH: i32 = -3;

type Ch = u16;
const M_QUOTE: Ch = 0x8000;
const M_PROTECT: Ch = 0x4000;
const M_ASCII: Ch = 0x00ff;
const M_ALL: Ch = b'*' as Ch | M_QUOTE;
const M_END: Ch = b']' as Ch | M_QUOTE;
const M_NOT: Ch = b'!' as Ch | M_QUOTE;
const M_ONE: Ch = b'?' as Ch | M_QUOTE;
const M_RNG: Ch = b'-' as Ch | M_QUOTE;
const M_SET: Ch = b'[' as Ch | M_QUOTE;
const M_CLASS: Ch = b':' as Ch | M_QUOTE;

const CCLASSES: [&str; 12] = [
    "alnum", "alpha", "blank", "cntrl", "digit", "graph", "lower", "print", "punct", "space",
    "upper", "xdigit",
];

fn cclass_match(idx: Ch, k: Ch) -> bool {
    if k > 0x7f {
        return false;
    }
    let c = k as u8;
    match idx {
        0 => c.is_ascii_alphanumeric(),
        1 => c.is_ascii_alphabetic(),
        2 => c == b' ' || c == b'\t',
        3 => c.is_ascii_control(),
        4 => c.is_ascii_digit(),
        5 => c.is_ascii_graphic(),
        6 => c.is_ascii_lowercase(),
        7 => c.is_ascii_graphic() || c == b' ',
        8 => c.is_ascii_punctuation(),
        9 => matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r'),
        10 => c.is_ascii_uppercase(),
        11 => c.is_ascii_hexdigit(),
        _ => false,
    }
}

fn ismeta(c: Ch) -> bool {
    c & M_QUOTE != 0
}

struct Glob {
    flags: i64,
    paths: Vec<Vec<u8>>,
}

impl Glob {
    fn run(&mut self, pattern: &[u8]) -> i32 {
        let mut buf: Vec<Ch> = Vec::with_capacity(pattern.len());
        if self.flags & GLOB_NOESCAPE != 0 {
            buf.extend(pattern.iter().map(|&b| b as Ch));
        } else {
            let mut i = 0;
            while i < pattern.len() {
                let c = pattern[i];
                i += 1;
                if c == b'\\' {
                    if i < pattern.len() {
                        buf.push(pattern[i] as Ch | M_PROTECT);
                        i += 1;
                    } else {
                        buf.push(b'\\' as Ch | M_PROTECT);
                    }
                } else {
                    buf.push(c as Ch);
                }
            }
        }
        if self.flags & GLOB_BRACE != 0 {
            self.globexp1(&buf)
        } else {
            self.glob0(&buf)
        }
    }

    fn globexp1(&mut self, pattern: &[Ch]) -> i32 {
        const LB: Ch = b'{' as Ch;
        const RB: Ch = b'}' as Ch;
        if pattern == [LB, RB] {
            return self.glob0(pattern);
        }
        if let Some(i) = pattern.iter().position(|&c| c == LB) {
            return self.globexp2(i, pattern);
        }
        self.glob0(pattern)
    }

    fn globexp2(&mut self, brace: usize, pattern: &[Ch]) -> i32 {
        const LB: Ch = b'{' as Ch;
        const RB: Ch = b'}' as Ch;
        const LBK: Ch = b'[' as Ch;
        const RBK: Ch = b']' as Ch;
        const COMMA: Ch = b',' as Ch;
        let at = |i: usize| -> Ch { pattern.get(i).copied().unwrap_or(0) };
        let prefix = &pattern[..brace];
        // find the balanced closing brace
        let mut depth = 0i32;
        let mut pe = brace + 1;
        while at(pe) != 0 {
            let c = at(pe);
            if c == LBK {
                let pm = pe;
                pe += 1;
                while at(pe) != RBK && at(pe) != 0 {
                    pe += 1;
                }
                if at(pe) == 0 {
                    pe = pm;
                }
            } else if c == LB {
                depth += 1;
            } else if c == RB {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            pe += 1;
        }
        if depth != 0 || at(pe) == 0 {
            return self.glob0(prefix);
        }
        let mut depth = 0i32;
        let mut pl = brace + 1;
        let mut pm = brace + 1;
        while pm <= pe {
            let c = at(pm);
            if c == LBK {
                let pb = pm;
                pm += 1;
                while at(pm) != RBK && at(pm) != 0 {
                    pm += 1;
                }
                if at(pm) == 0 {
                    pm = pb;
                }
            } else if c == LB {
                depth += 1;
            } else if (c == RB && depth == 0) || (c == COMMA && depth == 0) {
                let mut np: Vec<Ch> = prefix.to_vec();
                np.extend_from_slice(&pattern[pl..pm]);
                np.extend_from_slice(&pattern[pe + 1..]);
                let rv = self.globexp1(&np);
                if rv != 0 && rv != GLOB_NOMATCH {
                    return rv;
                }
                pl = pm + 1;
            } else if c == RB {
                depth -= 1;
            }
            pm += 1;
        }
        0
    }

    fn glob0(&mut self, pattern: &[Ch]) -> i32 {
        let at = |i: usize| -> Ch { pattern.get(i).copied().unwrap_or(0) };
        let oldpathc = self.paths.len();
        let mut out: Vec<Ch> = Vec::with_capacity(pattern.len());
        let mut i = 0usize;
        let mut magchar = false;
        while i < pattern.len() && pattern[i] != 0 {
            let c = pattern[i];
            i += 1;
            match c {
                x if x == b'[' as Ch => {
                    // (php 8.5.8's bundled glob does not treat a leading '^'
                    // as negation; only '!'.)
                    let c2 = at(i);
                    if c2 == b'!' as Ch {
                        i += 1;
                    }
                    let has_close =
                        at(i) != 0 && pattern[(i + 1).min(pattern.len())..].contains(&(b']' as Ch));
                    if !has_close {
                        out.push(b'[' as Ch);
                        if c2 == b'!' as Ch {
                            i -= 1;
                        }
                        continue;
                    }
                    out.push(M_SET);
                    if c2 == b'!' as Ch {
                        out.push(M_NOT);
                    }
                    let mut c = at(i);
                    i += 1;
                    loop {
                        if c == b'[' as Ch && at(i) == b':' as Ch {
                            let mut err;
                            loop {
                                err = self.charclass(pattern, &mut i, &mut out);
                                if err != 0 {
                                    break;
                                }
                                c = at(i);
                                i += 1;
                                if !(c == b'[' as Ch && at(i) == b':' as Ch) {
                                    break;
                                }
                            }
                            if err == -1 && self.flags & GLOB_NOCHECK == 0 {
                                return GLOB_NOMATCH;
                            }
                            if c == b']' as Ch {
                                break;
                            }
                        }
                        out.push(c & M_ASCII);
                        if at(i) == b'-' as Ch {
                            let c3 = at(i + 1);
                            if c3 != b']' as Ch {
                                out.push(M_RNG);
                                out.push(c3 & M_ASCII);
                                i += 2;
                            }
                        }
                        c = at(i);
                        i += 1;
                        if c == b']' as Ch {
                            break;
                        }
                    }
                    magchar = true;
                    out.push(M_END);
                }
                x if x == b'?' as Ch => {
                    magchar = true;
                    out.push(M_ONE);
                }
                x if x == b'*' as Ch => {
                    magchar = true;
                    if out.last() != Some(&M_ALL) {
                        out.push(M_ALL);
                    }
                }
                _ => out.push(c & M_ASCII),
            }
        }
        if magchar {
            self.flags |= GLOB_MAGCHAR;
        }
        if !out.is_empty() {
            let mut pathbuf: Vec<Ch> = Vec::new();
            let err = self.glob2(&mut pathbuf, &out);
            if err != 0 {
                return err;
            }
        }
        if self.paths.len() == oldpathc {
            if self.flags & GLOB_NOCHECK != 0 {
                self.paths.push(pattern.iter().map(|&c| c as u8).collect());
                return 0;
            }
            return GLOB_NOMATCH;
        }
        if self.flags & GLOB_NOSORT == 0 {
            self.paths[oldpathc..].sort();
        }
        0
    }

    /// `[:class:]` inside a bracket expression; 0 ok, 1 not a class, -1 invalid.
    fn charclass(&self, pattern: &[Ch], i: &mut usize, out: &mut Vec<Ch>) -> i32 {
        // *i points at ':' (the char after '[')
        let start = *i + 1;
        let rest = &pattern[start.min(pattern.len())..];
        let Some(colon) = rest.iter().position(|&c| c == b':' as Ch) else {
            return 1;
        };
        if rest.get(colon + 1).copied() != Some(b']' as Ch) {
            return 1;
        }
        let name: Vec<u8> = rest[..colon].iter().map(|&c| c as u8).collect();
        let Some(idx) = CCLASSES
            .iter()
            .position(|n| n.as_bytes() == name.as_slice())
        else {
            return -1;
        };
        out.push(M_CLASS);
        out.push(idx as Ch);
        *i = start + colon + 2;
        0
    }

    fn to_bytes(p: &[Ch]) -> Vec<u8> {
        p.iter().map(|&c| c as u8).collect()
    }

    fn glob2(&mut self, pathbuf: &mut Vec<Ch>, mut pattern: &[Ch]) -> i32 {
        loop {
            if pattern.is_empty() {
                let path = Self::to_bytes(pathbuf);
                let Some(sb) = crate::main::streams::sys_stat(&path, true) else {
                    return 0;
                };
                let mut path = path;
                if self.flags & GLOB_MARK != 0
                    && path.last() != Some(&b'/')
                    && (sb.is_dir
                        || (sb.is_link()
                            && crate::main::streams::sys_stat(&path, false)
                                .is_some_and(|s| s.is_dir)))
                {
                    path.push(b'/');
                }
                self.paths.push(path);
                return 0;
            }
            // next segment
            let mut anymeta = false;
            let mut p = 0;
            while p < pattern.len() && pattern[p] != b'/' as Ch {
                if ismeta(pattern[p]) {
                    anymeta = true;
                }
                p += 1;
            }
            if !anymeta {
                pathbuf.extend_from_slice(&pattern[..p]);
                pattern = &pattern[p..];
                while let Some(&c) = pattern.first() {
                    if c != b'/' as Ch {
                        break;
                    }
                    pathbuf.push(c);
                    pattern = &pattern[1..];
                }
            } else {
                return self.glob3(pathbuf, &pattern[..p], &pattern[p..]);
            }
        }
    }

    fn glob3(&mut self, pathbuf: &mut Vec<Ch>, pattern: &[Ch], rest: &[Ch]) -> i32 {
        let dir = if pathbuf.is_empty() {
            b".".to_vec()
        } else {
            Self::to_bytes(pathbuf)
        };
        // php passes no errfunc, so an unreadable directory is skipped even
        // with GLOB_ERR (the bundled glob only honours GLOB_ERR via errfunc).
        let Some(entries) = crate::main::streams::sys_readdir(&dir) else {
            return 0;
        };
        let base = pathbuf.len();
        for name in entries {
            if name.first() == Some(&b'.') && pattern.first() != Some(&(b'.' as Ch)) {
                continue;
            }
            let nm: Vec<Ch> = name.iter().map(|&b| b as Ch).collect();
            if !glob_match(&nm, pattern) {
                continue;
            }
            pathbuf.truncate(base);
            pathbuf.extend_from_slice(&nm);
            let err = self.glob2(pathbuf, rest);
            pathbuf.truncate(base);
            if err != 0 {
                return err;
            }
        }
        0
    }
}

/// php_glob's non-recursive `match()` (Russ Cox's algorithm).
fn glob_match(name: &[Ch], pat: &[Ch]) -> bool {
    let (mut n, mut p) = (0usize, 0usize);
    let mut next: Option<(usize, usize)> = None; // (pat, name)
    'outer: loop {
        let mut failed = false;
        while p < pat.len() {
            let c = pat[p];
            p += 1;
            match c {
                M_ALL => {
                    while p < pat.len() && pat[p] == M_ALL {
                        p += 1;
                    }
                    if p == pat.len() {
                        return true;
                    }
                    if n >= name.len() {
                        return false;
                    }
                    next = Some((p - 1, n + 1));
                }
                M_ONE => {
                    if n >= name.len() {
                        failed = true;
                        break;
                    }
                    n += 1;
                }
                M_SET => {
                    let mut ok = false;
                    if n >= name.len() {
                        failed = true;
                        break;
                    }
                    let k = name[n];
                    n += 1;
                    let negate = pat.get(p) == Some(&M_NOT);
                    if negate {
                        p += 1;
                    }
                    while let Some(&c) = pat.get(p) {
                        p += 1;
                        if c == M_END {
                            break;
                        }
                        if c == M_CLASS {
                            let idx = pat.get(p).copied().unwrap_or(0);
                            if (idx as usize) < CCLASSES.len() && cclass_match(idx, k) {
                                ok = true;
                            }
                            p += 1;
                        }
                        if pat.get(p) == Some(&M_RNG) {
                            let hi = pat.get(p + 1).copied().unwrap_or(0);
                            if c <= k && k <= hi {
                                ok = true;
                            }
                            p += 2;
                        } else if c == k {
                            ok = true;
                        }
                    }
                    if ok == negate {
                        failed = true;
                        break;
                    }
                }
                _ => {
                    if n >= name.len() || name[n] != c {
                        failed = true;
                        break;
                    }
                    n += 1;
                }
            }
        }
        if !failed && n >= name.len() {
            return true;
        }
        match next {
            Some((np, nn)) if nn <= name.len() => {
                p = np;
                n = nn;
                next = None;
                // re-arm: the M_ALL at `np` will set next again
                continue 'outer;
            }
            _ => return false,
        }
    }
}

/// php_glob(pattern, flags & PHP_GLOB_FLAGMASK): its return code and the
/// matches (`GLOB_ONLYDIR` is left to the caller, as php does).
pub(crate) fn php_glob(pattern: &[u8], flags: i64) -> (i32, Vec<Vec<u8>>) {
    let mut g = Glob {
        flags: flags & !GLOB_ONLYDIR,
        paths: Vec::new(),
    };
    let ret = g.run(pattern);
    (ret, g.paths)
}
