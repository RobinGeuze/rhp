//! main/streams/filter.c: buckets, brigades, the filter factory table and
//! `php_stream_filter_create`. The chains themselves live on the VM's
//! streams, which run them (php's streams.c read and write paths).

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use rhp_values::Value;

use crate::Host;

/// `PSFS_FLAG_*`: what a filter call is for.
pub const PSFS_FLAG_NORMAL: i32 = 0;
pub const PSFS_FLAG_FLUSH_INC: i32 = 1;
pub const PSFS_FLAG_FLUSH_CLOSE: i32 = 2;

/// `php_stream_filter_status_t`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    ErrFatal,
    FeedMe,
    PassOn,
}

impl Status {
    /// The `PSFS_*` value a user filter returns.
    pub fn from_long(n: i64) -> Status {
        match n {
            2 => Status::PassOn,
            1 => Status::FeedMe,
            _ => Status::ErrFatal,
        }
    }
}

type BucketList = RefCell<Vec<BucketRef>>;

/// `php_stream_bucket`: a piece of data, on at most one brigade.
pub struct Bucket {
    pub buf: Vec<u8>,
    brigade: Weak<BucketList>,
}

pub type BucketRef = Rc<RefCell<Bucket>>;

/// A new bucket on no brigade.
pub fn bucket_new(buf: Vec<u8>) -> BucketRef {
    Rc::new(RefCell::new(Bucket {
        buf,
        brigade: Weak::new(),
    }))
}

/// `php_stream_bucket_unlink`.
pub fn bucket_unlink(b: &BucketRef) {
    let brigade = std::mem::take(&mut b.borrow_mut().brigade);
    if let Some(list) = brigade.upgrade() {
        list.borrow_mut().retain(|x| !Rc::ptr_eq(x, b));
    }
}

/// Whether the bucket is on a brigade.
pub fn bucket_linked(b: &BucketRef) -> bool {
    b.borrow().brigade.strong_count() > 0
}

/// `php_stream_bucket_brigade`: a list of buckets.
#[derive(Clone, Default)]
pub struct Brigade(Rc<BucketList>);

impl Brigade {
    pub fn new() -> Brigade {
        Brigade::default()
    }

    pub fn is_empty(&self) -> bool {
        self.0.borrow().is_empty()
    }

    /// `php_stream_bucket_append` (a bucket already at the tail stays).
    pub fn append(&self, b: &BucketRef) {
        if self.0.borrow().last().is_some_and(|t| Rc::ptr_eq(t, b)) {
            return;
        }
        b.borrow_mut().brigade = Rc::downgrade(&self.0);
        self.0.borrow_mut().push(b.clone());
    }

    /// `php_stream_bucket_prepend`.
    pub fn prepend(&self, b: &BucketRef) {
        b.borrow_mut().brigade = Rc::downgrade(&self.0);
        self.0.borrow_mut().insert(0, b.clone());
    }

    /// Append a new bucket holding `buf`.
    pub fn append_data(&self, buf: Vec<u8>) {
        self.append(&bucket_new(buf));
    }

    /// Unlink and return the first bucket.
    pub fn pop(&self) -> Option<BucketRef> {
        let mut list = self.0.borrow_mut();
        if list.is_empty() {
            return None;
        }
        let b = list.remove(0);
        drop(list);
        b.borrow_mut().brigade = Weak::new();
        Some(b)
    }

    /// Unlink every bucket, returning them in order.
    pub fn take_all(&self) -> Vec<BucketRef> {
        let all = std::mem::take(&mut *self.0.borrow_mut());
        for b in &all {
            b.borrow_mut().brigade = Weak::new();
        }
        all
    }

    /// Drop every bucket.
    pub fn clear(&self) {
        self.take_all();
    }

    /// The total size of the data on the brigade.
    pub fn len_bytes(&self) -> usize {
        self.0.borrow().iter().map(|b| b.borrow().buf.len()).sum()
    }
}

/// `php_stream_filter_ops`: what a filter does with a call. `stream` is the
/// stream's resource (null while it is being destroyed); `consumed` is
/// `None` where php passes NULL.
pub trait FilterOps {
    fn filter(
        &mut self,
        host: &mut dyn Host,
        stream: &Value,
        inb: &Brigade,
        outb: &Brigade,
        consumed: Option<&mut usize>,
        flags: i32,
    ) -> Status;

    /// The filter's dtor (a user filter's `onClose`).
    fn dtor(&mut self, _host: &mut dyn Host) {}
}

/// `php_stream_filter`: an instance on a stream's chain.
pub struct Filter {
    pub ops: Box<dyn FilterOps>,
    /// Its `stream filter` resource id, if it got one.
    pub res: Option<u32>,
}

/// A filter factory's `create_filter`: `None` when the filter can't be made
/// (bad parameters, a user filter's `onCreate` returning false).
pub type Factory = fn(&mut dyn Host, &[u8], Option<&Value>) -> Option<Box<dyn FilterOps>>;

/// The filters php registers at startup, in registration order
/// (`stream_get_filters` lists them so).
fn global_factories() -> &'static [(&'static str, Factory)] {
    use crate::ext::standard::filters;
    &[
        ("zlib.*", crate::ext::zlib::zlib_filter::create),
        ("bzip2.*", crate::ext::bz2::bz2_filter::create),
        ("convert.iconv.*", crate::ext::iconv::iconv_filter::create),
        ("string.rot13", filters::create_rot13),
        ("string.toupper", filters::create_toupper),
        ("string.tolower", filters::create_tolower),
        ("convert.*", filters::create_convert),
        ("consumed", filters::create_consumed),
        ("dechunk", filters::create_dechunk),
    ]
}

thread_local! {
    /// The patterns `stream_filter_register` added this request (php's
    /// volatile copy of the filter hash), in order.
    static VOLATILE: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) };
}

/// Forget the request's registered filters.
pub fn reset_state() {
    VOLATILE.with(|v| v.borrow_mut().clear());
}

/// `stream_get_filters()`: every registered pattern.
pub fn filter_names() -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = global_factories()
        .iter()
        .map(|(n, _)| n.as_bytes().to_vec())
        .collect();
    VOLATILE.with(|v| out.extend(v.borrow().iter().cloned()));
    out
}

/// `php_stream_filter_register_factory_volatile` for a user filter: false
/// when the pattern is taken.
pub fn register_volatile(pattern: &[u8]) -> bool {
    if lookup(pattern).is_some() {
        return false;
    }
    VOLATILE.with(|v| v.borrow_mut().push(pattern.to_vec()));
    true
}

fn lookup(pattern: &[u8]) -> Option<Factory> {
    if let Some((_, f)) = global_factories()
        .iter()
        .find(|(n, _)| n.as_bytes() == pattern)
    {
        return Some(*f);
    }
    VOLATILE
        .with(|v| v.borrow().iter().any(|n| n == pattern))
        .then_some(crate::ext::standard::user_filters::create as Factory)
}

/// The patterns php tries for `name`: itself, then each `prefix.*` from the
/// longest prefix down ("a.b.c" -> "a.b.*", "a.*").
pub fn wildcards(name: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut end = name.len();
    while let Some(p) = name[..end].iter().rposition(|&c| c == b'.') {
        let mut w = name[..=p].to_vec();
        w.push(b'*');
        out.push(w);
        end = p;
    }
    out
}

/// `php_stream_filter_create`: warns (under the running function) and
/// returns `None` when no factory makes the filter.
pub fn create(host: &mut dyn Host, name: &[u8], params: Option<&Value>) -> Option<Filter> {
    let mut found = false;
    let mut ops = None;
    if let Some(f) = lookup(name) {
        found = true;
        ops = f(host, name, params);
    } else {
        // php's message depends on the last pattern looked up
        for w in wildcards(name) {
            let f = lookup(&w);
            found = f.is_some();
            if let Some(f) = f {
                ops = f(host, name, params);
                if ops.is_some() {
                    break;
                }
            }
        }
    }
    match ops {
        Some(ops) => Some(Filter { ops, res: None }),
        None => {
            let what = if found {
                "Unable to create or locate filter"
            } else {
                "Unable to locate filter"
            };
            let mut msg = format!("{}(): {what} \"", host.active_function_name()).into_bytes();
            msg.extend_from_slice(name);
            msg.push(b'"');
            raise(host, crate::E_WARNING, &msg);
            None
        }
    }
}

/// A warning from inside a filter, under the running function.
pub fn filter_warning(host: &mut dyn Host, level: i64, msg: &str) {
    let m = format!("{}(): {msg}", host.active_function_name());
    raise(host, level, m.as_bytes());
}

/// php_error_docref from filter code: shown even while an exception is
/// pending (php's C carries on with EG(exception) set).
pub fn raise(host: &mut dyn Host, level: i64, msg: &[u8]) {
    let held = host.take_pending_throw();
    host.raise_diagnostic(level, msg);
    if let Some(t) = held {
        host.set_pending_throw(t);
    }
}
