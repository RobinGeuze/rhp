//! Zend/zend_virtual_cwd.c's realpath cache: what tsrm_realpath_r found
//! for each absolute path it resolved, kept for realpath_cache_ttl
//! seconds. php keeps it per thread under ZTS (per process otherwise),
//! across requests; so does rhp, for the requests a thread serves. It is
//! visible to scripts: a removed symlink still resolves until
//! clearstatcache(true) (or an unlink, rename or rmdir) drops it, and
//! realpath_cache_get()/realpath_cache_size() show it.
//!
//! It is engine memory (php allocates it with malloc), so it is left out
//! of the request's memory accounting.

use std::cell::RefCell;

/// php's bucket count.
const BUCKETS: usize = 1024;

/// `sizeof(realpath_cache_bucket)` on a 64-bit POSIX build, which php
/// counts in realpath_cache_size().
const BUCKET_SIZE: i64 = 48;

pub struct Entry {
    pub key: u64,
    pub path: Vec<u8>,
    pub realpath: Vec<u8>,
    pub is_dir: bool,
    pub expires: i64,
}

impl Entry {
    fn size(&self) -> i64 {
        entry_size(&self.path, &self.realpath)
    }
}

/// The bytes php counts for an entry.
fn entry_size(path: &[u8], realpath: &[u8]) -> i64 {
    let mut n = BUCKET_SIZE + path.len() as i64 + 1;
    if realpath != path {
        n += realpath.len() as i64 + 1;
    }
    n
}

struct Cache {
    /// Each chain newest first, as php links them.
    buckets: Vec<Vec<Entry>>,
    size: i64,
    /// realpath_cache_size (0: off).
    limit: i64,
    ttl: i64,
    /// open_basedir was set when the request started: php turns the
    /// cache off.
    disabled: bool,
    /// The settings the entries were made under. php's are fixed for the
    /// process; a request on this thread with others starts empty.
    made_with: (i64, i64, bool),
}

impl Cache {
    const fn new() -> Cache {
        Cache {
            buckets: Vec::new(),
            size: 0,
            limit: 4096 * 1024,
            ttl: 120,
            disabled: false,
            made_with: (4096 * 1024, 120, false),
        }
    }

    fn enabled(&self) -> bool {
        self.limit != 0 && !self.disabled
    }

    fn settings(&self) -> (i64, i64, bool) {
        (self.limit, self.ttl, self.disabled)
    }

    /// Drop the entries made under other settings.
    fn check_settings(&mut self) {
        let now = self.settings();
        if self.made_with != now {
            self.clean();
            self.made_with = now;
        }
    }

    fn clean(&mut self) {
        rhp_values::mem::uncounted(|| self.buckets.clear());
        self.size = 0;
    }
}

thread_local! {
    static CACHE: RefCell<Cache> = const { RefCell::new(Cache::new()) };
}

/// php's realpath_cache_key (FNV-1 over the bytes). It xors in a C
/// `char`, which is signed on some targets (x86-64, Apple arm64).
fn key(path: &[u8]) -> u64 {
    let mut h: u64 = 2166136261;
    for &b in path {
        h = h.wrapping_mul(16777619);
        h ^= b as std::ffi::c_char as u64;
    }
    h
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// A new request: the default settings (the startup ini sets them
/// next); the entries stay.
pub(crate) fn reset_state() {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let fresh = Cache::new();
        c.limit = fresh.limit;
        c.ttl = fresh.ttl;
        c.disabled = fresh.disabled;
    });
}

/// The startup ini: realpath_cache_size, realpath_cache_ttl, and whether
/// open_basedir is set (which turns the cache off).
pub(crate) fn configure(limit: Option<i64>, ttl: Option<i64>, open_basedir: Option<bool>) {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(l) = limit {
            c.limit = l;
        }
        if let Some(t) = ttl {
            c.ttl = t;
        }
        if let Some(d) = open_basedir {
            c.disabled = d;
        }
    });
}

/// realpath_cache_find: the cached `(realpath, is_dir)` of `path`,
/// dropping the expired entries met on the way.
pub(crate) fn find(path: &[u8]) -> Option<(Vec<u8>, bool)> {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if !c.enabled() || c.buckets.is_empty() {
            return None;
        }
        c.check_settings();
        if c.buckets.is_empty() {
            return None;
        }
        let k = key(path);
        let n = (k % BUCKETS as u64) as usize;
        let t = now();
        let ttl = c.ttl;
        let mut freed = 0;
        let mut found = None;
        let chain = &mut c.buckets[n];
        let mut i = 0;
        while i < chain.len() {
            if ttl != 0 && chain[i].expires < t {
                let e = chain.remove(i);
                freed += e.size();
                rhp_values::mem::uncounted(|| drop(e));
                continue;
            }
            let e = &chain[i];
            if e.key == k && e.path == path {
                found = Some((e.realpath.clone(), e.is_dir));
                break;
            }
            i += 1;
        }
        c.size -= freed;
        found
    })
}

/// realpath_cache_add, when the entry fits under the size limit.
pub(crate) fn add(path: &[u8], realpath: &[u8], is_dir: bool) {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if !c.enabled() {
            return;
        }
        c.check_settings();
        let k = key(path);
        let size = entry_size(path, realpath);
        if c.size + size > c.limit {
            return;
        }
        let expires = now() + c.ttl;
        rhp_values::mem::uncounted(|| {
            let e = Entry {
                key: k,
                path: path.to_vec(),
                realpath: realpath.to_vec(),
                is_dir,
                expires,
            };
            if c.buckets.is_empty() {
                c.buckets.resize_with(BUCKETS, Vec::new);
            }
            c.buckets[(k % BUCKETS as u64) as usize].insert(0, e);
        });
        c.size += size;
    });
}

/// realpath_cache_del: forget `path`'s entry.
pub fn del(path: &[u8]) {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.buckets.is_empty() {
            return;
        }
        let k = key(path);
        let chain = &mut c.buckets[(k % BUCKETS as u64) as usize];
        if let Some(i) = chain.iter().position(|e| e.key == k && e.path == path) {
            let e = chain.remove(i);
            c.size -= e.size();
            rhp_values::mem::uncounted(|| drop(e));
        }
    });
}

/// realpath_cache_clean: forget everything.
pub fn clean() {
    CACHE.with(|c| c.borrow_mut().clean());
}

/// realpath_cache_size(): the bytes php counts for the entries.
pub fn size() -> i64 {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.check_settings();
        c.size
    })
}

/// The entries in php's order (bucket by bucket, each newest first), for
/// realpath_cache_get().
pub fn entries<R>(f: impl FnOnce(&mut dyn Iterator<Item = &Entry>) -> R) -> R {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.check_settings();
        f(&mut c.buckets.iter().flatten())
    })
}
