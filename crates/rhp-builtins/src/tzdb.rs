//! Timezone database access for the date builtins.
//!
//! Zones are loaded from the system TZif files (`/usr/share/zoneinfo`) and
//! queried with timelib's semantics (ext/date/lib/parse_tz.c,
//! parse_posix.c, tm2unixtime.c): explicit transitions first, then the POSIX
//! TZ footer for instants after the last transition.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const SECS_PER_DAY: i64 = 86400;

#[derive(Clone, Debug)]
pub(crate) struct TType {
    pub(crate) off: i64,
    pub(crate) dst: bool,
    pub(crate) abbr: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
enum RuleKind {
    /// `Jn`: 1..365, Feb 29 never counted.
    JulianNoFeb29(i64),
    /// `n`: 0..365, Feb 29 counted.
    Julian(i64),
    /// `Mm.w.d`.
    Mwd(i64, i64, i64),
}

#[derive(Clone, Copy, Debug)]
struct Rule {
    kind: RuleKind,
    /// Seconds after local midnight (may be negative or > 24h).
    time: i64,
}

#[derive(Clone, Debug)]
struct Posix {
    std: TType,
    dst: Option<(TType, Rule, Rule)>,
}

#[derive(Debug)]
pub(crate) struct Zone {
    trans: Vec<i64>,
    idx: Vec<usize>,
    types: Vec<TType>,
    posix: Option<Posix>,
}

thread_local! {
    /// Pure data cache: zone name (lowercased) → parsed zone.
    static CACHE: RefCell<HashMap<Vec<u8>, Option<Rc<Zone>>>> = RefCell::new(HashMap::new());
}

/// Resolve `id` case-insensitively under the zoneinfo directory.
fn resolve(id: &[u8]) -> Option<std::path::PathBuf> {
    let s = std::str::from_utf8(id).ok()?;
    if s.is_empty()
        || s.starts_with('/')
        || s.split('/').any(|c| c.is_empty() || c == "." || c == "..")
    {
        return None;
    }
    let mut dir = std::path::PathBuf::from("/usr/share/zoneinfo");
    let comps: Vec<&str> = s.split('/').collect();
    for (i, comp) in comps.iter().enumerate() {
        let rd = std::fs::read_dir(&dir).ok()?;
        let mut found = None;
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n == *comp {
                found = Some(e.path());
                break;
            }
            if found.is_none() && n.eq_ignore_ascii_case(comp) {
                found = Some(e.path());
            }
        }
        dir = found?;
        let is_dir = dir.is_dir();
        if i + 1 == comps.len() {
            if is_dir {
                return None;
            }
            return Some(dir);
        }
        if !is_dir {
            return None;
        }
    }
    None
}

/// Load (and cache) a zone by identifier. `UTC` resolves like any other id.
pub(crate) fn load(id: &[u8]) -> Option<Rc<Zone>> {
    let key = id.to_ascii_lowercase();
    if let Some(z) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return z;
    }
    let z = resolve(id).and_then(|path| {
        let data = std::fs::read(path).ok()?;
        parse_tzif(&data).map(Rc::new)
    });
    let z = z.or_else(|| {
        // The zoneinfo tree may lack a plain "UTC" file; synthesize it.
        id.eq_ignore_ascii_case(b"UTC").then(|| {
            Rc::new(Zone {
                trans: vec![],
                idx: vec![],
                types: vec![TType {
                    off: 0,
                    dst: false,
                    abbr: b"UTC".to_vec(),
                }],
                posix: None,
            })
        })
    });
    CACHE.with(|c| c.borrow_mut().insert(key, z.clone()));
    z
}

fn be32(b: &[u8], at: usize) -> Option<i64> {
    let s = b.get(at..at + 4)?;
    Some(i32::from_be_bytes([s[0], s[1], s[2], s[3]]) as i64)
}

fn be64(b: &[u8], at: usize) -> Option<i64> {
    let s = b.get(at..at + 8)?;
    let mut a = [0u8; 8];
    a.copy_from_slice(s);
    Some(i64::from_be_bytes(a))
}

fn parse_tzif(d: &[u8]) -> Option<Zone> {
    if !d.starts_with(b"TZif") {
        return None;
    }
    let counts = |at: usize| -> Option<[usize; 6]> {
        let mut c = [0usize; 6];
        for (i, v) in c.iter_mut().enumerate() {
            *v = be32(d, at + 20 + i * 4)? as u32 as usize;
        }
        Some(c)
    };
    let version = d[4];
    let mut c = counts(0)?;
    let mut base = 44;
    let mut tsize = 4;
    if version >= b'2' {
        let [isut, isstd, leap, timecnt, typecnt, charcnt] = c;
        base += timecnt * 4 + timecnt + typecnt * 6 + charcnt + leap * 8 + isstd + isut;
        if d.get(base..base + 4) != Some(b"TZif") {
            return None;
        }
        c = counts(base)?;
        base += 44;
        tsize = 8;
    }
    let [isut, isstd, leap, timecnt, typecnt, charcnt] = c;
    let mut p = base;
    let mut trans = Vec::with_capacity(timecnt);
    for _ in 0..timecnt {
        trans.push(if tsize == 8 { be64(d, p)? } else { be32(d, p)? });
        p += tsize;
    }
    let mut idx = Vec::with_capacity(timecnt);
    for _ in 0..timecnt {
        idx.push(*d.get(p)? as usize);
        p += 1;
    }
    let mut raw = Vec::with_capacity(typecnt);
    for _ in 0..typecnt {
        raw.push((be32(d, p)?, *d.get(p + 4)? != 0, *d.get(p + 5)? as usize));
        p += 6;
    }
    let chars = d.get(p..p + charcnt)?;
    p += charcnt;
    let types: Vec<TType> = raw
        .into_iter()
        .map(|(off, dst, ai)| {
            let a = chars.get(ai..).unwrap_or(&[]);
            let end = a.iter().position(|&c| c == 0).unwrap_or(a.len());
            TType {
                off,
                dst,
                abbr: a[..end].to_vec(),
            }
        })
        .collect();
    if types.is_empty() || idx.iter().any(|&i| i >= types.len()) {
        return None;
    }
    p += leap * (tsize + 4) + isstd + isut;
    let mut posix = None;
    if tsize == 8 && d.get(p) == Some(&b'\n') {
        let rest = &d[p + 1..];
        if let Some(e) = rest.iter().position(|&c| c == b'\n') {
            posix = parse_posix(&rest[..e]);
        }
    }
    Some(Zone {
        trans,
        idx,
        types,
        posix,
    })
}

// ---------------------------------------------------------------------------
// POSIX TZ strings
// ---------------------------------------------------------------------------

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn name(&mut self) -> Option<Vec<u8>> {
        if self.peek() == Some(b'<') {
            let st = self.i + 1;
            let e = self.s[st..].iter().position(|&c| c == b'>')? + st;
            self.i = e + 1;
            return Some(self.s[st..e].to_vec());
        }
        let st = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.i += 1;
        }
        (self.i - st >= 3).then(|| self.s[st..self.i].to_vec())
    }
    fn num(&mut self) -> Option<i64> {
        let st = self.i;
        let mut n = 0i64;
        while let Some(c) = self.peek().filter(|c| c.is_ascii_digit()) {
            n = n * 10 + (c - b'0') as i64;
            self.i += 1;
        }
        (self.i > st).then_some(n)
    }
    /// `[+-]hh[:mm[:ss]]` → seconds (sign as written).
    fn hms(&mut self) -> Option<i64> {
        let sign = match self.peek() {
            Some(b'-') => {
                self.i += 1;
                -1
            }
            Some(b'+') => {
                self.i += 1;
                1
            }
            _ => 1,
        };
        let mut v = self.num()? * 3600;
        if self.peek() == Some(b':') {
            self.i += 1;
            v += self.num()? * 60;
            if self.peek() == Some(b':') {
                self.i += 1;
                v += self.num()?;
            }
        }
        Some(sign * v)
    }
    fn rule(&mut self) -> Option<Rule> {
        let kind = match self.peek()? {
            b'J' => {
                self.i += 1;
                RuleKind::JulianNoFeb29(self.num()?)
            }
            b'M' => {
                self.i += 1;
                let m = self.num()?;
                (self.peek() == Some(b'.')).then_some(())?;
                self.i += 1;
                let w = self.num()?;
                (self.peek() == Some(b'.')).then_some(())?;
                self.i += 1;
                let d = self.num()?;
                RuleKind::Mwd(m, w, d)
            }
            _ => RuleKind::Julian(self.num()?),
        };
        let time = if self.peek() == Some(b'/') {
            self.i += 1;
            self.hms()?
        } else {
            7200
        };
        Some(Rule { kind, time })
    }
}

fn parse_posix(s: &[u8]) -> Option<Posix> {
    if s.is_empty() {
        return None;
    }
    let mut p = P { s, i: 0 };
    let std_abbr = p.name()?;
    let std_off = -p.hms()?;
    let std = TType {
        off: std_off,
        dst: false,
        abbr: std_abbr,
    };
    if p.peek().is_none() {
        return Some(Posix { std, dst: None });
    }
    let dst_abbr = p.name()?;
    let dst_off = if p.peek().is_some_and(|c| c != b',') {
        -p.hms()?
    } else {
        std_off + 3600
    };
    (p.peek() == Some(b',')).then_some(())?;
    p.i += 1;
    let start = p.rule()?;
    (p.peek() == Some(b',')).then_some(())?;
    p.i += 1;
    let end = p.rule()?;
    let dst = TType {
        off: dst_off,
        dst: true,
        abbr: dst_abbr,
    };
    Some(Posix {
        std,
        dst: Some((dst, start, end)),
    })
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

const MONTH_LEN: [[i64; 12]; 2] = [
    [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31],
    [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31],
];

/// parse_posix.c `calc_transition`: seconds since the start of `year`.
fn calc_transition(r: &Rule, year: i64) -> i64 {
    let leap = is_leap(year);
    match r.kind {
        RuleKind::JulianNoFeb29(days) => {
            let mut v = days - 1;
            if leap && days >= 60 {
                v += 1;
            }
            v * SECS_PER_DAY
        }
        RuleKind::Julian(days) => days * SECS_PER_DAY,
        RuleKind::Mwd(month, week, dow_want) => {
            let m1 = (month + 9) % 12 + 1;
            let yy0 = if month <= 2 { year - 1 } else { year };
            let yy1 = yy0 / 100;
            let yy2 = yy0 % 100;
            let mut dow = ((26 * m1 - 2) / 10 + 1 + yy2 + yy2 / 4 + yy1 / 4 - 2 * yy1) % 7;
            if dow < 0 {
                dow += 7;
            }
            let mut d = dow_want - dow;
            if d < 0 {
                d += 7;
            }
            let ml = &MONTH_LEN[leap as usize];
            let mi = (month - 1).clamp(0, 11) as usize;
            for _ in 1..week {
                if d + 7 >= ml[mi] {
                    break;
                }
                d += 7;
            }
            let mut v = d * SECS_PER_DAY;
            for l in ml.iter().take(mi) {
                v += l * SECS_PER_DAY;
            }
            v
        }
    }
}

fn ts_at_start_of_year(year: i64) -> i64 {
    let cly = |y: i64| {
        let y = y - 1;
        y.div_euclid(4) - y.div_euclid(100) + y.div_euclid(400)
    };
    SECS_PER_DAY * ((year - 1970) * 365 + cly(year) - cly(1970))
}

fn year_of(ts: i64) -> i64 {
    // civil year of a UTC timestamp
    let z = ts.div_euclid(SECS_PER_DAY) + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    if mp >= 10 {
        y + 1
    } else {
        y
    }
}

impl Zone {
    fn posix_offset(&self, p: &Posix, ts: i64) -> (TType, i64) {
        let last = self.trans.last().copied().unwrap_or(i64::MIN);
        let Some((dst, start, end)) = &p.dst else {
            return (p.std.clone(), last);
        };
        let year = year_of(ts);
        let mut times: Vec<(i64, bool)> = Vec::with_capacity(6);
        for y in [year - 1, year, year + 1] {
            let ys = ts_at_start_of_year(y);
            let b = ys + calc_transition(start, y) + start.time - p.std.off;
            let e = ys + calc_transition(end, y) + end.time - dst.off;
            if b < e {
                times.push((b, true));
                times.push((e, false));
            } else {
                times.push((e, false));
                times.push((b, true));
            }
        }
        for i in 1..times.len() {
            if ts < times[i].0 {
                let (t, isdst) = times[i - 1];
                return (if isdst { dst.clone() } else { p.std.clone() }, t);
            }
        }
        (p.std.clone(), last)
    }

    /// `timelib_fetch_timezone_offset`: the type in effect at `ts` and the
    /// transition time that started it (`i64::MIN` if none).
    pub(crate) fn offset_info(&self, ts: i64) -> (TType, i64) {
        if self.trans.is_empty() {
            if let Some(p) = &self.posix {
                let (t, _) = self.posix_offset(p, ts);
                return (t, i64::MIN);
            }
            return (self.types[0].clone(), i64::MIN);
        }
        if ts < self.trans[0] {
            return (self.types[0].clone(), i64::MIN);
        }
        let n = self.trans.len();
        if ts >= self.trans[n - 1] {
            if let Some(p) = &self.posix {
                return self.posix_offset(p, ts);
            }
            return (self.types[self.idx[n - 1]].clone(), self.trans[n - 1]);
        }
        let (mut l, mut r) = (0usize, n - 1);
        while r - l > 1 {
            let m = (l + r) / 2;
            if ts < self.trans[m] {
                r = m;
            } else {
                l = m;
            }
        }
        (self.types[self.idx[l]].clone(), self.trans[l])
    }

    pub(crate) fn at(&self, ts: i64) -> TType {
        self.offset_info(ts).0
    }

    /// tm2unixtime.c `do_adjust_timezone` (TIMELIB_ZONETYPE_ID, have_zone):
    /// convert a local wall-clock second count (`local` = civil seconds as if
    /// UTC) to a Unix timestamp. `hint_dst` is the struct's dst flag.
    pub(crate) fn local_to_utc(&self, local: i64, hint_dst: bool) -> i64 {
        let (cur, _) = self.offset_info(local);
        let current_offset = cur.off;
        let current_is_dst = cur.dst;
        let (after, after_tt) = self.offset_info(local - current_offset);
        let mut actual_offset = after.off;
        let mut actual_tt = after_tt;
        if current_offset == after.off {
            if current_offset >= 0 && hint_dst && !current_is_dst {
                let (e, ett) = self.offset_info(local - current_offset - 7200);
                if e.off != after.off && local - e.off < after_tt {
                    actual_offset = e.off;
                    actual_tt = ett;
                }
            } else if current_offset <= 0 && current_is_dst && !hint_dst {
                let (l, ltt) = self.offset_info(local - current_offset + 7200);
                if l.off != after.off && local - l.off >= ltt {
                    actual_offset = l.off;
                    actual_tt = ltt;
                }
            }
        }
        let in_transition = actual_tt != i64::MIN
            && (local - actual_offset) >= actual_tt + (current_offset - actual_offset)
            && (local - actual_offset) < actual_tt;
        if current_offset != actual_offset && !in_transition {
            local - actual_offset
        } else {
            local - current_offset
        }
    }
}

/// The effective default zone (date_default_timezone_set → date.timezone →
/// UTC); `None` means plain UTC.
pub(crate) fn default_zone() -> Option<Rc<Zone>> {
    let name = crate::runtime_fns::default_timezone_name();
    if name == b"UTC" {
        return None;
    }
    load(&name)
}

/// Offset/dst/abbr of the default zone at `ts`.
pub(crate) fn local_type(ts: i64) -> TType {
    match default_zone() {
        Some(z) => z.at(ts),
        None => TType {
            off: 0,
            dst: false,
            abbr: b"UTC".to_vec(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_rules() {
        let p = parse_posix(b"EST5EDT,M3.2.0,M11.1.0").unwrap();
        assert_eq!(p.std.off, -18000);
        let z = Zone {
            trans: vec![],
            idx: vec![],
            types: vec![p.std.clone()],
            posix: Some(p),
        };
        // 2024-03-10 07:00 UTC is the US spring-forward instant.
        assert_eq!(z.at(1710054000).off, -14400);
        assert_eq!(z.at(1710053999).off, -18000);
        assert_eq!(z.local_to_utc(1710028800, false), 1710046800);
        let p = parse_posix(b"<-03>3").unwrap();
        assert_eq!(p.std.abbr, b"-03");
        assert_eq!(p.std.off, -10800);
    }
}
