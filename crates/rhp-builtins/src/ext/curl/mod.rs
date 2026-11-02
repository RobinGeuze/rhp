//! The `curl` extension (php-src `ext/curl`): `CurlHandle` / `CurlMultiHandle`
//! / `CurlShareHandle` objects and the `curl_*` function family, with php 8.5's
//! argument validation and error texts.
//!
//! Handle state lives in a per-thread registry keyed by the PHP object's
//! identity (a weak pointer guards against address reuse), so the objects stay
//! property-less like php's (`var_dump` shows `object(CurlHandle)#1 (0) {}`).
//! Transfers are performed by [`net`] (ureq for http/https, direct file reads
//! for `file://`). `curl_multi_*` runs each added transfer on its own worker
//! thread — real concurrency — and delivers the results (header/body handlers,
//! callbacks, `curl_multi_info_read` messages) on the VM thread from
//! `curl_multi_exec`. `curl_version()` reports the libcurl that php 8.5.8 links
//! here, so version-dependent output (composer's `lib-curl*` platform
//! packages, its User-Agent) is identical.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::{Rc, Weak};
use std::sync::mpsc;
use std::time::Duration;

use rhp_values::{conv, ArrayKey, ObjectData, ObjectHandle, PhpArray, Value};

use crate::{Builtin, Host, Thrown};

mod consts;
mod net;

use net::{Info, Outcome, Spec};

// ---------------------------------------------------------------------------
// option / info numbers
// ---------------------------------------------------------------------------

const OPT_PORT: i64 = 3;
const OPT_TIMEOUT: i64 = 13;
const OPT_INFILESIZE: i64 = 14;
const OPT_VERBOSE: i64 = 41;
const OPT_HEADER: i64 = 42;
const OPT_NOPROGRESS: i64 = 43;
const OPT_NOBODY: i64 = 44;
const OPT_FAILONERROR: i64 = 45;
const OPT_UPLOAD: i64 = 46;
const OPT_POST: i64 = 47;
const OPT_FOLLOWLOCATION: i64 = 52;
const OPT_AUTOREFERER: i64 = 58;
const OPT_PUT: i64 = 54;
const OPT_SSL_VERIFYPEER: i64 = 64;
const OPT_MAXREDIRS: i64 = 68;
const OPT_UNRESTRICTED_AUTH: i64 = 105;
const OPT_CONNECTTIMEOUT: i64 = 78;
const OPT_HTTPGET: i64 = 80;
const OPT_SSL_VERIFYHOST: i64 = 81;
const OPT_HTTP_VERSION: i64 = 84;
const OPT_DNS_USE_GLOBAL_CACHE: i64 = 91;
const OPT_IPRESOLVE: i64 = 113;
const OPT_TIMEOUT_MS: i64 = 155;
const OPT_CONNECTTIMEOUT_MS: i64 = 156;
const OPT_POSTREDIR: i64 = 161;
const OPT_PROTOCOLS: i64 = 181;
const OPT_REDIR_PROTOCOLS: i64 = 182;
const OPT_FILE: i64 = 10001;
const OPT_URL: i64 = 10002;
const OPT_PROXY: i64 = 10004;
const OPT_USERPWD: i64 = 10005;
const OPT_RANGE: i64 = 10007;
const OPT_INFILE: i64 = 10009;
const OPT_POSTFIELDS: i64 = 10015;
const OPT_REFERER: i64 = 10016;
const OPT_USERAGENT: i64 = 10018;
const OPT_COOKIE: i64 = 10022;
const OPT_HTTPHEADER: i64 = 10023;
const OPT_WRITEHEADER: i64 = 10029;
const OPT_CUSTOMREQUEST: i64 = 10036;
const OPT_STDERR: i64 = 10037;
const OPT_CAINFO: i64 = 10065;
const OPT_CAPATH: i64 = 10097;
const OPT_SHARE: i64 = 10100;
const OPT_ENCODING: i64 = 10102;
const OPT_PRIVATE: i64 = 10103;
const OPT_USERNAME: i64 = 10173;
const OPT_PASSWORD: i64 = 10174;
const OPT_WRITEFUNCTION: i64 = 20011;
const OPT_READFUNCTION: i64 = 20012;
const OPT_PROGRESSFUNCTION: i64 = 20056;
const OPT_HEADERFUNCTION: i64 = 20079;
const OPT_DEBUGFUNCTION: i64 = 20094;
const OPT_FNMATCH_FUNCTION: i64 = 20200;
const OPT_XFERINFOFUNCTION: i64 = 20219;
const OPT_PREREQFUNCTION: i64 = 20312;
const OPT_SSH_HOSTKEYFUNCTION: i64 = 20316;
const OPT_RETURNTRANSFER: i64 = 19913;
const OPT_BINARYTRANSFER: i64 = 19914;
const OPT_SAFE_UPLOAD: i64 = -1;
const INFO_HEADER_OUT: i64 = 2;
const INFO_PRIVATE: i64 = 0x100000 + 21;
const INFO_CERTINFO: i64 = 0x400000 + 34;

/// Options php's `_php_curl_setopt` routes to the string-option path.
/// (Generated from interface.c's "String options" case list; note
/// CURLOPT_SSLENGINE_DEFAULT (90) and CURLOPT_TRANSFER_ENCODING (207) sit there
/// despite their long-range numbers.) The file-path options that follow the
/// open_basedir check are included too: without open_basedir they behave the
/// same.
const STRING_OPTS: &[i64] = &[
    10065, 10097, 10022, 10077, 10062, 10004, 10006, 10016, 10086, 10089, 90, 10087, 10026, 10088,
    10083, 10018, 10135, 10147, 10162, 10176, 10175, 10177, 10179, 10186, 10191, 10192, 10206,
    10205, 10204, 207, 10211, 10217, 10224, 10230, 10235, 10236, 10238, 10262, 10246, 10247, 10260,
    10258, 10263, 10259, 10254, 10255, 10256, 10257, 10252, 10253, 10251, 10264, 10266, 10277,
    10276, 10287, 10289, 10296, 10298, 10305, 10311, 10318, 10319,
    // file options (COOKIEFILE .. SSH_KNOWNHOSTS)
    10031, 10082, 10076, 10025, 10118, 10153, 10152, 10169, 10170, 10183,
];

const CURLPROTO_ALL: i64 = !0;

// ---------------------------------------------------------------------------
// value helpers
// ---------------------------------------------------------------------------

fn deref(v: &Value) -> Value {
    match v {
        Value::Ref(r) => r.borrow().clone(),
        o => o.clone(),
    }
}

/// `zend_zval_value_name()`: the type named in "X given" messages.
fn value_name(host: &dyn Host, v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(true) => "true".into(),
        Value::Bool(false) => "false".into(),
        Value::Int(_) => "int".into(),
        Value::Float(_) => "float".into(),
        Value::Str(_) => "string".into(),
        Value::Array(_) => "array".into(),
        Value::Closure(_) => "Closure".into(),
        Value::Object(_) => host
            .object_class_name(v)
            .map(|n| String::from_utf8_lossy(&n).into_owned())
            .unwrap_or_else(|| "object".into()),
        Value::Resource(_) => "resource".into(),
        Value::Ref(r) => value_name(host, &r.borrow()),
    }
}

/// `Z_PARAM_LONG` coercion (non-strict mode).
fn arg_long(host: &dyn Host, f: &str, n: usize, pname: &str, v: &Value) -> Result<i64, Thrown> {
    let v = deref(v);
    match &v {
        Value::Int(i) => Ok(*i),
        Value::Bool(b) => Ok(*b as i64),
        Value::Null => Ok(0),
        Value::Float(x) if x.fract() == 0.0 && x.is_finite() => Ok(*x as i64),
        Value::Str(_) if conv::is_numeric(&v) => Ok(conv::to_int(&v)),
        _ => Err(Thrown::type_error(format!(
            "{f}(): Argument #{n} (${pname}) must be of type int, {} given",
            value_name(host, &v)
        ))),
    }
}

fn arg_float(host: &dyn Host, f: &str, n: usize, pname: &str, v: &Value) -> Result<f64, Thrown> {
    let v = deref(v);
    match &v {
        Value::Int(i) => Ok(*i as f64),
        Value::Float(x) => Ok(*x),
        Value::Bool(b) => Ok(*b as i64 as f64),
        Value::Null => Ok(0.0),
        Value::Str(_) if conv::is_numeric(&v) => Ok(conv::to_float(&v)),
        _ => Err(Thrown::type_error(format!(
            "{f}(): Argument #{n} (${pname}) must be of type float, {} given",
            value_name(host, &v)
        ))),
    }
}

fn set_ref(slot: &Value, v: Value) {
    if let Value::Ref(r) = slot {
        *r.borrow_mut() = v;
    }
}

fn akey(s: &str) -> ArrayKey {
    ArrayKey::from_bytes(s.as_bytes())
}

// ---------------------------------------------------------------------------
// handle state + registry
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum WriteMethod {
    Stdout,
    File,
    Return,
    User,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum HeaderMethod {
    Ignore,
    File,
    User,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum MultiState {
    Idle,
    Pending,
    Running(u64),
}

#[derive(Clone)]
struct Easy {
    url: String,
    write: WriteMethod,
    write_stream: Option<Value>,
    write_fn: Option<Value>,
    header: HeaderMethod,
    header_stream: Option<Value>,
    header_fn: Option<Value>,
    read_fn: Option<Value>,
    progress_fn: Option<Value>,
    xferinfo_fn: Option<Value>,
    other_fns: HashMap<i64, Value>,
    noprogress: bool,
    include_header: bool,
    nobody: bool,
    post: bool,
    upload: bool,
    infile: Option<Value>,
    postfields: Option<Value>,
    httpheader: Vec<Vec<u8>>,
    custom_request: Option<String>,
    follow: bool,
    max_redirs: i64,
    timeout_ms: u64,
    connect_timeout_ms: u64,
    proxy: Option<String>,
    userpwd: Option<String>,
    username: Option<String>,
    password: Option<String>,
    useragent: Option<String>,
    referer: Option<String>,
    cookie: Option<String>,
    range: Option<String>,
    encoding: Option<String>,
    ipresolve: i64,
    protocols: i64,
    redir_protocols: i64,
    failonerror: bool,
    cainfo: Option<String>,
    capath: Option<String>,
    private: Option<Value>,
    share: Option<Value>,
    /// Generic storage for options with no transfer effect here.
    misc: HashMap<i64, Value>,
    // results of the last transfer
    info: Option<Info>,
    errno: i64,
    error: String,
    ret_buf: Option<Vec<u8>>,
    multi: MultiState,
    in_callback: bool,
}

impl Easy {
    fn new() -> Easy {
        Easy {
            url: String::new(),
            write: WriteMethod::Stdout,
            write_stream: None,
            write_fn: None,
            header: HeaderMethod::Ignore,
            header_stream: None,
            header_fn: None,
            read_fn: None,
            progress_fn: None,
            xferinfo_fn: None,
            other_fns: HashMap::new(),
            noprogress: true,
            include_header: false,
            nobody: false,
            post: false,
            upload: false,
            infile: None,
            postfields: None,
            httpheader: Vec::new(),
            custom_request: None,
            follow: false,
            max_redirs: 20,
            timeout_ms: 0,
            connect_timeout_ms: 0,
            proxy: None,
            userpwd: None,
            username: None,
            password: None,
            useragent: None,
            referer: None,
            cookie: None,
            range: None,
            encoding: None,
            ipresolve: 0,
            protocols: CURLPROTO_ALL,
            redir_protocols: net::PROTO_HTTP | net::PROTO_HTTPS | 4 | 8,
            failonerror: false,
            cainfo: None,
            capath: None,
            private: None,
            share: None,
            misc: HashMap::new(),
            info: None,
            errno: 0,
            error: String::new(),
            ret_buf: None,
            multi: MultiState::Idle,
            in_callback: false,
        }
    }

    fn method(&self) -> String {
        if let Some(m) = &self.custom_request {
            return m.clone();
        }
        if self.nobody {
            "HEAD".into()
        } else if self.upload {
            "PUT".into()
        } else if self.post {
            "POST".into()
        } else {
            "GET".into()
        }
    }

    fn cleanup(&mut self) {
        self.errno = 0;
        self.error.clear();
        self.ret_buf = None;
    }
}

struct Multi {
    handles: Vec<Value>,
    msgs: VecDeque<(i64, Value)>,
    errno: i64,
    tx: mpsc::Sender<(u64, Outcome)>,
    rx: mpsc::Receiver<(u64, Outcome)>,
    ready: Vec<(u64, Outcome)>,
    running: HashMap<u64, Value>,
    next_ticket: u64,
}

impl Multi {
    fn new() -> Multi {
        let (tx, rx) = mpsc::channel();
        Multi {
            handles: Vec::new(),
            msgs: VecDeque::new(),
            errno: 0,
            tx,
            rx,
            ready: Vec::new(),
            running: HashMap::new(),
            next_ticket: 1,
        }
    }
}

#[derive(Default)]
struct Share {
    errno: i64,
}

struct Entry<T> {
    obj: Weak<RefCell<ObjectData>>,
    data: Rc<RefCell<T>>,
}

#[derive(Default)]
struct Registry {
    easy: HashMap<usize, Entry<Easy>>,
    multi: HashMap<usize, Entry<Multi>>,
    share: HashMap<usize, Entry<Share>>,
}

impl Registry {
    fn sweep(&mut self) {
        self.easy.retain(|_, e| e.obj.strong_count() > 0);
        self.multi.retain(|_, e| e.obj.strong_count() > 0);
        self.share.retain(|_, e| e.obj.strong_count() > 0);
    }
}

thread_local! {
    static REG: RefCell<Registry> = RefCell::new(Registry::default());
}

/// Forget every handle and pending callback exception (a new request).
pub(crate) fn reset_state() {
    REG.with(|r| *r.borrow_mut() = Registry::default());
    PENDING.with(|p| *p.borrow_mut() = None);
}

fn obj_handle(v: &Value) -> Option<ObjectHandle> {
    match v {
        Value::Object(h) => Some(h.clone()),
        Value::Ref(r) => obj_handle(&r.borrow()),
        _ => None,
    }
}

fn key_of(h: &ObjectHandle) -> usize {
    Rc::as_ptr(h) as *const () as usize
}

fn lookup<T>(map: &HashMap<usize, Entry<T>>, h: &ObjectHandle) -> Option<Rc<RefCell<T>>> {
    let e = map.get(&key_of(h))?;
    let live = e.obj.upgrade()?;
    if Rc::ptr_eq(&live, h) {
        Some(e.data.clone())
    } else {
        None
    }
}

fn new_object(host: &mut dyn Host, class: &str) -> Result<(Value, ObjectHandle), Thrown> {
    let v = host
        .instantiate(class.as_bytes(), Vec::new())
        // A host without object support (the tree-walking oracle) skips curl.
        .ok_or_else(|| Thrown::unsupported("curl", format!("cannot instantiate {class}")))?;
    let h = obj_handle(&v).ok_or_else(|| Thrown::error("Error", "curl: bad object"))?;
    // The handle classes use zend_objects_not_comparable.
    rhp_semantics::ops::set_not_comparable_hook(is_curl_object);
    Ok((v, h))
}

/// Is `h` a CurlHandle / CurlMultiHandle / CurlShareHandle of ours?
fn is_curl_object(h: &ObjectHandle) -> bool {
    REG.with(|r| {
        let r = r.borrow();
        lookup(&r.easy, h).is_some()
            || lookup(&r.multi, h).is_some()
            || lookup(&r.share, h).is_some()
    })
}

fn register_easy(host: &mut dyn Host, easy: Easy) -> Result<Value, Thrown> {
    let (v, h) = new_object(host, "CurlHandle")?;
    REG.with(|r| {
        let mut r = r.borrow_mut();
        r.sweep();
        r.easy.insert(
            key_of(&h),
            Entry {
                obj: Rc::downgrade(&h),
                data: Rc::new(RefCell::new(easy)),
            },
        );
    });
    Ok(v)
}

fn type_err(host: &dyn Host, f: &str, n: usize, pname: &str, class: &str, v: &Value) -> Thrown {
    Thrown::type_error(format!(
        "{f}(): Argument #{n} (${pname}) must be of type {class}, {} given",
        value_name(host, &deref(v))
    ))
}

fn easy_arg(
    host: &dyn Host,
    f: &str,
    n: usize,
    v: &Value,
) -> Result<(ObjectHandle, Rc<RefCell<Easy>>), Thrown> {
    if let Some(h) = obj_handle(v) {
        if let Some(e) = REG.with(|r| lookup(&r.borrow().easy, &h)) {
            return Ok((h, e));
        }
    }
    Err(type_err(host, f, n, "handle", "CurlHandle", v))
}

fn multi_arg(
    host: &dyn Host,
    f: &str,
    v: &Value,
) -> Result<(ObjectHandle, Rc<RefCell<Multi>>), Thrown> {
    if let Some(h) = obj_handle(v) {
        if let Some(e) = REG.with(|r| lookup(&r.borrow().multi, &h)) {
            return Ok((h, e));
        }
    }
    Err(type_err(host, f, 1, "multi_handle", "CurlMultiHandle", v))
}

fn share_arg(host: &dyn Host, f: &str, v: &Value) -> Result<Rc<RefCell<Share>>, Thrown> {
    if let Some(h) = obj_handle(v) {
        if let Some(e) = REG.with(|r| lookup(&r.borrow().share, &h)) {
            return Ok(e);
        }
    }
    Err(type_err(host, f, 1, "share_handle", "CurlShareHandle", v))
}

/// Is `v` a `CurlHandle` / `CurlMultiHandle` this extension created? Both
/// classes install `curl_cast_object`, so `(int) $handle` is the object id
/// (used by the VM's `(int)` cast and `intval()`).
pub fn is_curl_handle(v: &Value) -> bool {
    obj_handle(v)
        .map(|h| {
            REG.with(|r| {
                let r = r.borrow();
                lookup(&r.easy, &h).is_some() || lookup(&r.multi, &h).is_some()
            })
        })
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// error strings
// ---------------------------------------------------------------------------

fn easy_strerror(code: i64) -> &'static str {
    match code {
        0 => "No error",
        1 => "Unsupported protocol",
        2 => "Failed initialization",
        3 => "URL using bad/illegal format or missing URL",
        4 => "A requested feature, protocol or option was not found built-in in this libcurl due to a build-time decision.",
        5 => "Could not resolve proxy name",
        6 => "Could not resolve hostname",
        7 => "Could not connect to server",
        8 => "Weird server reply",
        9 => "Access denied to remote resource",
        10 => "FTP: The server failed to connect to data port",
        11 => "FTP: unknown PASS reply",
        12 => "FTP: Accepting server connect has timed out",
        13 => "FTP: unknown PASV reply",
        14 => "FTP: unknown 227 response format",
        15 => "FTP: cannot figure out the host in the PASV response",
        16 => "Error in the HTTP2 framing layer",
        17 => "FTP: could not set file type",
        18 => "Transferred a partial file",
        19 => "FTP: could not retrieve (RETR failed) the specified file",
        21 => "Quote command returned error",
        22 => "HTTP response code said error",
        23 => "Failed writing received data to disk/application",
        25 => "Upload failed (at start/before it took off)",
        26 => "Failed to open/read local data from file/application",
        27 => "Out of memory",
        28 => "Timeout was reached",
        30 => "FTP: command PORT failed",
        31 => "FTP: command REST failed",
        33 => "Requested range was not delivered by the server",
        35 => "SSL connect error",
        36 => "Could not resume download",
        37 => "Could not read a file:// file",
        38 => "LDAP: cannot bind",
        39 => "LDAP: search failed",
        42 => "Operation was aborted by an application callback",
        43 => "A libcurl function was given a bad argument",
        45 => "Failed binding local connection end",
        47 => "Number of redirects hit maximum amount",
        48 => "An unknown option was passed in to libcurl",
        49 => "Malformed option provided in a setopt",
        52 => "Server returned nothing (no headers, no data)",
        53 => "SSL crypto engine not found",
        54 => "Can not set SSL crypto engine as default",
        55 => "Failed sending data to the peer",
        56 => "Failure when receiving data from the peer",
        58 => "Problem with the local SSL certificate",
        59 => "Could not use specified SSL cipher",
        60 => "SSL peer certificate or SSH remote key was not OK",
        61 => "Unrecognized or bad HTTP Content or Transfer-Encoding",
        63 => "Maximum file size exceeded",
        64 => "Requested SSL level failed",
        65 => "Send failed since rewinding of the data stream failed",
        66 => "Failed to initialize SSL crypto engine",
        67 => "Login denied",
        68 => "TFTP: File Not Found",
        69 => "TFTP: Access Violation",
        70 => "Disk full or allocation exceeded",
        71 => "TFTP: Illegal operation",
        72 => "TFTP: Unknown transfer ID",
        73 => "Remote file already exists",
        74 => "TFTP: No such user",
        77 => "Problem with the SSL CA cert (path? access rights?)",
        78 => "Remote file not found",
        79 => "Error in the SSH layer",
        80 => "Failed to shut down the SSL connection",
        81 => "Socket not ready for send/recv",
        82 => "Failed to load CRL file (path? access rights?, format?)",
        83 => "Issuer check against peer certificate failed",
        84 => "FTP: The server did not accept the PRET command.",
        85 => "RTSP CSeq mismatch or invalid CSeq",
        86 => "RTSP session error",
        87 => "Unable to parse FTP file list",
        88 => "Chunk callback failed",
        89 => "The max connection limit is reached",
        90 => "SSL public key does not match pinned public key",
        91 => "SSL server certificate status verification FAILED",
        92 => "Stream error in the HTTP/2 framing layer",
        93 => "API function called from within callback",
        94 => "An authentication function returned an error",
        95 => "HTTP/3 error",
        96 => "QUIC connection error",
        97 => "proxy handshake error",
        98 => "SSL Client Certificate required",
        99 => "Unrecoverable error in select/poll",
        100 => "A value or data field grew larger than allowed",
        101 => "ECH attempted but failed",
        _ => "Unknown error",
    }
}

fn multi_strerror(code: i64) -> &'static str {
    match code {
        -1 => "Please call curl_multi_perform() soon",
        0 => "No error",
        1 => "Invalid multi handle",
        2 => "Invalid easy handle",
        3 => "Out of memory",
        4 => "Internal error",
        5 => "Invalid socket argument",
        6 => "Unknown option",
        7 => "The easy handle is already added to a multi handle",
        8 => "API function called from within callback",
        9 => "Wakeup is unavailable or failed",
        10 => "A libcurl function was given a bad argument",
        11 => "Operation was aborted by an application callback",
        12 => "Unrecoverable error in select/poll",
        _ => "Unknown error",
    }
}

fn share_strerror(code: i64) -> &'static str {
    match code {
        0 => "No error",
        1 => "Unknown share option",
        2 => "Share currently in use",
        3 => "Invalid share handle",
        4 => "Out of memory",
        5 => "Feature not enabled in this library",
        _ => "CURLSHcode unknown",
    }
}

// ---------------------------------------------------------------------------
// curl_init / setopt
// ---------------------------------------------------------------------------

fn check_nul(f: &str, s: &[u8]) -> Result<(), Thrown> {
    if s.contains(&0) {
        Err(Thrown::error(
            "ValueError",
            format!("{f}(): cURL option must not contain any null bytes"),
        ))
    } else {
        Ok(())
    }
}

fn fn_curl_init(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let mut easy = Easy::new();
    if let Some(u) = args.first() {
        match deref(u) {
            Value::Null => {}
            Value::Array(_) | Value::Object(_) | Value::Closure(_) | Value::Resource(_) => {
                return Err(Thrown::type_error(format!(
                    "curl_init(): Argument #1 ($url) must be of type ?string, {} given",
                    value_name(host, &deref(u))
                )))
            }
            v => {
                let s = conv::to_string_bytes(&v);
                check_nul("curl_init", &s)?;
                easy.url = String::from_utf8_lossy(&s).into_owned();
            }
        }
    }
    register_easy(host, easy)
}

/// `zend_is_callable_ex()`'s error text for a non-callable value.
fn callable_error(v: &Value) -> String {
    match deref(v) {
        Value::Str(s) => {
            let s = String::from_utf8_lossy(s.as_bytes()).into_owned();
            if let Some((c, _)) = s.split_once("::") {
                format!("class \"{c}\" not found")
            } else {
                format!("function \"{s}\" not found or invalid function name")
            }
        }
        Value::Array(a) => {
            if a.len() != 2 {
                "array callback must have exactly two members".into()
            } else {
                "first array member is not a valid class name or object".into()
            }
        }
        _ => "no array or string given".into(),
    }
}

fn opt_name_for_fn(option: i64) -> &'static str {
    match option {
        OPT_WRITEFUNCTION => "CURLOPT_WRITEFUNCTION",
        OPT_HEADERFUNCTION => "CURLOPT_HEADERFUNCTION",
        OPT_READFUNCTION => "CURLOPT_READFUNCTION",
        OPT_PROGRESSFUNCTION => "CURLOPT_PROGRESSFUNCTION",
        OPT_XFERINFOFUNCTION => "CURLOPT_XFERINFOFUNCTION",
        OPT_FNMATCH_FUNCTION => "CURLOPT_FNMATCH_FUNCTION",
        OPT_DEBUGFUNCTION => "CURLOPT_DEBUGFUNCTION",
        OPT_PREREQFUNCTION => "CURLOPT_PREREQFUNCTION",
        OPT_SSH_HOSTKEYFUNCTION => "CURLOPT_SSH_HOSTKEYFUNCTION",
        _ => "",
    }
}

fn slist_name(option: i64) -> Option<&'static str> {
    Some(match option {
        10104 => "CURLOPT_HTTP200ALIASES",
        OPT_HTTPHEADER => "CURLOPT_HTTPHEADER",
        10039 => "CURLOPT_POSTQUOTE",
        10093 => "CURLOPT_PREQUOTE",
        10028 => "CURLOPT_QUOTE",
        10070 => "CURLOPT_TELNETOPTIONS",
        10187 => "CURLOPT_MAIL_RCPT",
        10203 => "CURLOPT_RESOLVE",
        10228 => "CURLOPT_PROXYHEADER",
        10243 => "CURLOPT_CONNECT_TO",
        _ => return None,
    })
}

fn is_valid_option(option: i64) -> bool {
    use std::sync::OnceLock;
    static SET: OnceLock<std::collections::HashSet<i64>> = OnceLock::new();
    SET.get_or_init(|| {
        consts::CONSTANTS
            .iter()
            .filter(|(n, _)| n.starts_with("CURLOPT_"))
            .map(|(_, v)| *v)
            .chain([INFO_HEADER_OUT])
            .collect()
    })
    .contains(&option)
}

// ---------------------------------------------------------------------------
// libcurl's own argument checks
// ---------------------------------------------------------------------------

/// Would libcurl 8.x's `curl_easy_setopt` (setopt.c, with the features of the
/// libcurl php 8.5.8 links: no NTLM, RTMP, SMB, c-ares or TLS-SRP) refuse this
/// long value? CURLE_BAD_FUNCTION_ARGUMENT / CURLE_NOT_BUILT_IN make php's
/// curl_setopt() return false.
fn libcurl_rejects_long(option: i64, v: i64) -> bool {
    let u = v as u64;
    match option {
        // timeouts, sizes and counters: no negatives
        13 | 19 | 20 | 71 | 78 | 98 | 112 | 114 | 155 | 156 | 178 | 193 | 194 | 212 | 214 | 215
        | 227 | 271 | 280 | 281 | 288 | 314 | 326 | 30117 | 30145 | 30146 => v < 0,
        // -1 means "unset"/"forever"
        14 | 21 | 68 | 92 | 321 | 30115 => v < -1,
        // enumerations
        51 | 110 | 113 | 129 | 141 | 154 => !(0..=2).contains(&v),
        33 | 52 | 119 | 138 => !(0..=3).contains(&v),
        101 => !(0..=7).contains(&v),
        189 => !(1..=11).contains(&v),
        84 => !matches!(v, 0..=5 | 30 | 31),
        // ports
        3 | 59 | 139 | 140 => !(0..=65535).contains(&v),
        // CURLOPT_NEW_FILE_PERMS / NEW_DIRECTORY_PERMS
        159 | 160 => !(0..=0o777).contains(&v),
        // CURLOPT_ADDRESS_SCOPE
        171 => v < 0 || v > u32::MAX as i64,
        // CURLOPT_SSLVERSION / PROXY_SSLVERSION: CURL_SSLVERSION_* in the low
        // 16 bits (SSLv2/SSLv3 refused), CURL_SSLVERSION_MAX_* above.
        32 | 250 => {
            let lo = u & 0xffff;
            let hi = u & 0xffff_0000;
            lo >= 8 || lo == 2 || lo == 3 || hi >= 8 << 16
        }
        // CURLOPT_HTTPAUTH / PROXYAUTH: DIGEST_IE folds into DIGEST, NTLM is
        // not built in; some auth bit (0..30) must remain.
        107 | 111 => {
            if u == 0 {
                return false;
            }
            let mut a = u;
            if a & 16 != 0 {
                a = (a | 2) & !16;
            }
            a &= !8;
            a & 0x7fff_ffff == 0
        }
        // CURLOPT_SOCKS5_AUTH: only CURLAUTH_BASIC | CURLAUTH_GSSAPI
        267 => u & !5 != 0,
        // CURLOPT_ALTSVC_CTRL
        286 => v == 0,
        // CURLOPT_SSL_FALSESTART: not supported by the TLS backend
        233 => true,
        _ => false,
    }
}

/// The protocols this libcurl is built with (curl_version()['protocols']),
/// as CURLPROTO_* bits.
const BUILT_PROTOCOLS: &[(&str, i64)] = &[
    ("dict", 512),
    ("file", 1024),
    ("ftp", 4),
    ("ftps", 8),
    ("gopher", 33554432),
    ("gophers", 536870912),
    ("http", 1),
    ("https", 2),
    ("imap", 4096),
    ("imaps", 8192),
    ("ldap", 128),
    ("ldaps", 256),
    ("mqtt", 268435456),
    ("mqtts", 1 << 32),
    ("pop3", 16384),
    ("pop3s", 32768),
    ("rtsp", 262144),
    ("scp", 16),
    ("sftp", 32),
    ("smtp", 65536),
    ("smtps", 131072),
    ("telnet", 64),
    ("tftp", 2048),
    ("ws", 1073741824),
    ("wss", 2147483648),
];

/// libcurl's protocol2num() for CURLOPT_PROTOCOLS_STR / REDIR_PROTOCOLS_STR:
/// "all", or a comma list of built-in scheme names (case-insensitive, empty
/// items skipped). `None` = CURLE_BAD_FUNCTION_ARGUMENT.
fn protocols_from_str(s: &[u8]) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    if s.eq_ignore_ascii_case(b"all") {
        return Some(CURLPROTO_ALL);
    }
    let mut bits = 0i64;
    for item in s.split(|&b| b == b',') {
        if item.is_empty() {
            continue;
        }
        bits |= BUILT_PROTOCOLS
            .iter()
            .find(|(n, _)| n.as_bytes().eq_ignore_ascii_case(item))?
            .1;
    }
    Some(bits)
}

/// libcurl's refusals for string options.
fn libcurl_rejects_str(option: i64, s: &[u8]) -> bool {
    match option {
        // CURLOPT_PROTOCOLS_STR / REDIR_PROTOCOLS_STR
        10318 | 10319 => protocols_from_str(s).is_none(),
        // CURLOPT_INTERFACE
        10062 => s.is_empty(),
        // CURLOPT_SSLENGINE: no OpenSSL engines are available
        10089 => !s.is_empty(),
        // CURLOPT_TLSAUTH_TYPE / PROXY_TLSAUTH_TYPE: only "SRP"
        10206 | 10253 => !s.eq_ignore_ascii_case(b"SRP"),
        // KRBLEVEL, DNS_SERVERS, DNS_INTERFACE, DNS_LOCAL_IP4/6: not built in
        10063 | 10211 | 10221 | 10222 | 10223 => true,
        _ => false,
    }
}

/// php's `zval_get_string()` for an option value: arrays warn "Array to
/// string conversion", objects go through `__toString` or throw.
fn zval_get_string(host: &mut dyn Host, v: &Value) -> Result<Vec<u8>, Thrown> {
    match deref(v) {
        Value::Array(_) => {
            host.emit_diagnostic(crate::E_WARNING, "Array to string conversion")?;
            Ok(b"Array".to_vec())
        }
        o @ Value::Object(_) => {
            let has = match &o {
                Value::Object(h) => {
                    let class = h.borrow().class;
                    host.class_has_method(class, b"__tostring")
                }
                _ => false,
            };
            if has {
                let mut cb = PhpArray::new();
                cb.push(o.clone());
                cb.push(Value::str("__toString"));
                let r = host.call_callable(&Value::Array(cb), &[])?;
                return Ok(conv::to_string_bytes(&deref(&r)));
            }
            Err(Thrown::error(
                "Error",
                format!(
                    "Object of class {} could not be converted to string",
                    value_name(host, &o)
                ),
            ))
        }
        Value::Closure(_) => Err(Thrown::error(
            "Error",
            "Object of class Closure could not be converted to string",
        )),
        other => Ok(conv::to_string_bytes(&other)),
    }
}

/// php's `zval_get_long()` for an option value (objects warn and give 1).
fn zval_get_long(host: &mut dyn Host, v: &Value) -> Result<i64, Thrown> {
    let v = deref(v);
    if matches!(v, Value::Object(_) | Value::Closure(_)) {
        host.emit_diagnostic(
            crate::E_WARNING,
            &format!(
                "Object of class {} could not be converted to int",
                value_name(host, &v)
            ),
        )?;
    }
    Ok(conv::to_int(&v))
}

/// `_php_curl_setopt`: apply one option. `Ok(false)` = php returns false.
fn setopt(
    host: &mut dyn Host,
    f: &str,
    easy: &Rc<RefCell<Easy>>,
    option: i64,
    value: &Value,
    is_array: bool,
) -> Result<bool, Thrown> {
    let value = deref(value);
    let (argn, pname) = if is_array {
        (2, "options")
    } else {
        (3, "value")
    };

    // callable options
    if matches!(
        option,
        OPT_WRITEFUNCTION
            | OPT_HEADERFUNCTION
            | OPT_READFUNCTION
            | OPT_PROGRESSFUNCTION
            | OPT_XFERINFOFUNCTION
            | OPT_FNMATCH_FUNCTION
            | OPT_DEBUGFUNCTION
            | OPT_PREREQFUNCTION
            | OPT_SSH_HOSTKEYFUNCTION
    ) {
        let cb = if matches!(value, Value::Null) {
            None
        } else if host.is_callable_value(&value) {
            Some(value.clone())
        } else {
            let mut e = easy.borrow_mut();
            match option {
                OPT_WRITEFUNCTION => e.write = WriteMethod::Stdout,
                OPT_HEADERFUNCTION => e.header = HeaderMethod::Ignore,
                _ => {}
            }
            drop(e);
            return Err(Thrown::type_error(format!(
                "{f}(): Argument #{argn} (${pname}) must be a valid callback for option {}, {}",
                opt_name_for_fn(option),
                callable_error(&value)
            )));
        };
        let mut e = easy.borrow_mut();
        match option {
            OPT_WRITEFUNCTION => {
                e.write = if cb.is_some() {
                    WriteMethod::User
                } else {
                    WriteMethod::Stdout
                };
                e.write_fn = cb;
            }
            OPT_HEADERFUNCTION => {
                e.header = if cb.is_some() {
                    HeaderMethod::User
                } else {
                    HeaderMethod::Ignore
                };
                e.header_fn = cb;
            }
            OPT_READFUNCTION => e.read_fn = cb,
            OPT_PROGRESSFUNCTION => e.progress_fn = cb,
            OPT_XFERINFOFUNCTION => e.xferinfo_fn = cb,
            _ => match cb {
                Some(c) => {
                    e.other_fns.insert(option, c);
                }
                None => {
                    e.other_fns.remove(&option);
                }
            },
        }
        return Ok(true);
    }

    if STRING_OPTS.contains(&option) {
        let s = zval_get_string(host, &value)?;
        check_nul(f, &s)?;
        if libcurl_rejects_str(option, &s) {
            return Ok(false);
        }
        let protocols = protocols_from_str(&s);
        let s = String::from_utf8_lossy(&s).into_owned();
        let mut e = easy.borrow_mut();
        match option {
            10318 => e.protocols = protocols.unwrap_or(CURLPROTO_ALL),
            10319 => e.redir_protocols = protocols.unwrap_or(CURLPROTO_ALL),
            OPT_CAINFO => e.cainfo = Some(s),
            OPT_CAPATH => e.capath = Some(s),
            OPT_COOKIE => e.cookie = Some(s),
            OPT_PROXY => e.proxy = Some(s),
            OPT_REFERER => e.referer = Some(s),
            OPT_USERAGENT => e.useragent = Some(s),
            _ => {
                e.misc.insert(option, Value::str(s));
            }
        }
        return Ok(true);
    }

    match option {
        OPT_URL => {
            let s = zval_get_string(host, &value)?;
            check_nul(f, &s)?;
            easy.borrow_mut().url = String::from_utf8_lossy(&s).into_owned();
        }
        OPT_SSL_VERIFYHOST => {
            let mut v = zval_get_long(host, &value)?;
            if v == 1 {
                crate::notice(
                    host,
                    f,
                    "CURLOPT_SSL_VERIFYHOST no longer accepts the value 1, value 2 will be used instead",
                )?;
                v = 2;
            }
            easy.borrow_mut().misc.insert(option, Value::Int(v));
        }
        OPT_RETURNTRANSFER => {
            easy.borrow_mut().write = if conv::to_bool(&value) {
                WriteMethod::Return
            } else {
                WriteMethod::Stdout
            };
        }
        OPT_BINARYTRANSFER | OPT_DNS_USE_GLOBAL_CACHE => {}
        OPT_SAFE_UPLOAD => {
            if !conv::to_bool(&value) {
                return Err(Thrown::error(
                    "ValueError",
                    format!("{f}(): Disabling safe uploads is no longer supported"),
                ));
            }
        }
        OPT_PRIVATE => easy.borrow_mut().private = Some(value.clone()),
        OPT_FILE | OPT_WRITEHEADER | OPT_INFILE | OPT_STDERR => {
            let stream = match &value {
                Value::Null => None,
                Value::Resource(r) if r.type_name == "stream" && host.stream_is_open(&value) => {
                    Some(value.clone())
                }
                Value::Resource(_) => {
                    return Err(Thrown::type_error(format!(
                        "{f}(): supplied resource is not a valid File-Handle resource"
                    )))
                }
                _ => {
                    return Err(Thrown::type_error(format!(
                        "{f}(): supplied argument is not a valid File-Handle resource"
                    )))
                }
            };
            // FILE / WRITEHEADER / STDERR: what->mode[0] != 'r' || what->mode[1] == '+'.
            if option != OPT_INFILE {
                if let Some(mode) = stream.as_ref().and_then(|s| host.stream_mode(s)) {
                    if mode.first() == Some(&b'r') && mode.get(1) != Some(&b'+') {
                        return Err(Thrown::error(
                            "ValueError",
                            format!("{f}(): The provided file handle must be writable"),
                        ));
                    }
                }
            }
            let mut e = easy.borrow_mut();
            match option {
                OPT_FILE => {
                    e.write = if stream.is_some() {
                        WriteMethod::File
                    } else {
                        WriteMethod::Stdout
                    };
                    e.write_stream = stream;
                }
                OPT_WRITEHEADER => {
                    e.header = if stream.is_some() {
                        HeaderMethod::File
                    } else {
                        HeaderMethod::Ignore
                    };
                    e.header_stream = stream;
                }
                OPT_INFILE => e.infile = stream,
                _ => {
                    match stream {
                        Some(s) => e.misc.insert(option, s),
                        None => e.misc.remove(&option),
                    };
                }
            }
        }
        OPT_POSTFIELDS => {
            let mut e = easy.borrow_mut();
            // libcurl: POSTFIELDS selects POST, but CURLOPT_NOBODY (HEAD) keeps
            // precedence for the effective method.
            e.post = true;
            e.upload = false;
            let pf = match &value {
                Value::Array(_) => value.clone(),
                other => {
                    drop(e);
                    let s = zval_get_string(host, other)?;
                    e = easy.borrow_mut();
                    Value::str(s)
                }
            };
            e.postfields = Some(pf);
        }
        OPT_CUSTOMREQUEST | OPT_RANGE | OPT_USERPWD | OPT_USERNAME | OPT_PASSWORD
        | OPT_ENCODING => {
            let s = if matches!(value, Value::Null) {
                None
            } else {
                let b = zval_get_string(host, &value)?;
                check_nul(f, &b)?;
                Some(String::from_utf8_lossy(&b).into_owned())
            };
            let mut e = easy.borrow_mut();
            match option {
                OPT_CUSTOMREQUEST => e.custom_request = s,
                OPT_RANGE => e.range = s,
                OPT_USERPWD => e.userpwd = s,
                OPT_USERNAME => e.username = s,
                OPT_PASSWORD => e.password = s,
                _ => e.encoding = s,
            }
        }
        OPT_SHARE => {
            if obj_handle(&value)
                .map(|h| REG.with(|r| lookup(&r.borrow().share, &h).is_some()))
                .unwrap_or(false)
            {
                easy.borrow_mut().share = Some(value.clone());
            }
        }
        INFO_HEADER_OUT => {
            easy.borrow_mut()
                .misc
                .insert(option, Value::Bool(conv::to_bool(&value)));
        }
        _ if slist_name(option).is_some() => {
            let Value::Array(a) = &value else {
                return Err(Thrown::type_error(format!(
                    "{f}(): The {} option must have an array value",
                    slist_name(option).unwrap_or("")
                )));
            };
            let mut lines: Vec<Vec<u8>> = Vec::with_capacity(a.len());
            for (_, v) in a.iter() {
                lines.push(zval_get_string(host, v)?);
            }
            let mut e = easy.borrow_mut();
            if option == OPT_HTTPHEADER {
                e.httpheader = lines;
            } else {
                e.misc.insert(option, value.clone());
            }
        }
        _ if is_valid_option(option) && option < 10000 || (30000..40000).contains(&option) => {
            let v = zval_get_long(host, &value)?;
            if libcurl_rejects_long(option, v) {
                return Ok(false);
            }
            let mut e = easy.borrow_mut();
            match option {
                OPT_HEADER => e.include_header = v != 0,
                OPT_NOPROGRESS => e.noprogress = v != 0,
                OPT_NOBODY => e.nobody = v != 0,
                OPT_POST => {
                    e.post = v != 0;
                    if v != 0 {
                        e.upload = false;
                    }
                }
                OPT_HTTPGET => {
                    if v != 0 {
                        e.post = false;
                        e.nobody = false;
                        e.upload = false;
                    }
                }
                OPT_UPLOAD | OPT_PUT => {
                    e.upload = v != 0;
                    if v != 0 {
                        e.nobody = false;
                    }
                }
                OPT_FAILONERROR => e.failonerror = v != 0,
                OPT_FOLLOWLOCATION => e.follow = v != 0,
                OPT_MAXREDIRS => e.max_redirs = v,
                OPT_TIMEOUT => e.timeout_ms = (v.max(0) as u64).saturating_mul(1000),
                OPT_TIMEOUT_MS => e.timeout_ms = v.max(0) as u64,
                OPT_CONNECTTIMEOUT => e.connect_timeout_ms = (v.max(0) as u64).saturating_mul(1000),
                OPT_CONNECTTIMEOUT_MS => e.connect_timeout_ms = v.max(0) as u64,
                OPT_IPRESOLVE => e.ipresolve = v,
                OPT_PROTOCOLS => e.protocols = v,
                OPT_REDIR_PROTOCOLS => e.redir_protocols = v,
                OPT_POSTREDIR => {
                    e.misc.insert(option, Value::Int(v & 7));
                }
                OPT_SSL_VERIFYPEER | OPT_HTTP_VERSION | OPT_VERBOSE | OPT_INFILESIZE | OPT_PORT => {
                    e.misc.insert(option, Value::Int(v));
                }
                _ => {
                    e.misc.insert(option, Value::Int(v));
                }
            }
        }
        _ if is_valid_option(option) => {
            // Remaining file-path / blob options: stored, string-coerced.
            let s = zval_get_string(host, &value)?;
            check_nul(f, &s)?;
            if libcurl_rejects_str(option, &s) {
                return Ok(false);
            }
            easy.borrow_mut().misc.insert(option, Value::str(s));
        }
        _ => {
            return Err(Thrown::error(
                "ValueError",
                if is_array {
                    format!("{f}(): Argument #2 ($options) must contain only valid cURL options")
                } else {
                    format!("{f}(): Argument #2 ($option) is not a valid cURL option")
                },
            ));
        }
    }
    Ok(true)
}

fn fn_curl_setopt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, easy) = easy_arg(host, "curl_setopt", 1, &args[0])?;
    let option = arg_long(host, "curl_setopt", 2, "option", &args[1])?;
    Ok(Value::Bool(setopt(
        host,
        "curl_setopt",
        &easy,
        option,
        &args[2],
        false,
    )?))
}

fn fn_curl_setopt_array(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, easy) = easy_arg(host, "curl_setopt_array", 1, &args[0])?;
    let Value::Array(a) = deref(&args[1]) else {
        return Err(Thrown::type_error(format!(
            "curl_setopt_array(): Argument #2 ($options) must be of type array, {} given",
            value_name(host, &deref(&args[1]))
        )));
    };
    for (k, v) in a.iter() {
        let ArrayKey::Int(option) = k else {
            return Err(Thrown::error(
                "ValueError",
                "curl_setopt_array(): Argument #2 ($options) contains an invalid cURL option",
            ));
        };
        if !setopt(host, "curl_setopt_array", &easy, *option, v, true)? {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

// ---------------------------------------------------------------------------
// transfers
// ---------------------------------------------------------------------------

fn random_boundary() -> String {
    let mut b = [0u8; 8];
    let _ = getrandom::getrandom(&mut b);
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("------------------------{hex}")
}

/// Build the request body for CURLOPT_POSTFIELDS (string → urlencoded as-is,
/// array → multipart/form-data with CURLFile parts read from disk).
fn build_body(host: &mut dyn Host, pf: &Value) -> (Vec<u8>, String) {
    match pf {
        Value::Array(a) if !a.is_empty() => {
            let boundary = random_boundary();
            let mut out = Vec::new();
            for (k, v) in a.iter() {
                let name = match k {
                    ArrayKey::Int(i) => i.to_string(),
                    ArrayKey::Str(s) => String::from_utf8_lossy(s.as_bytes()).into_owned(),
                };
                let v = deref(v);
                out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
                let class = if matches!(v, Value::Object(_)) {
                    host.object_class_name(&v)
                        .map(|c| String::from_utf8_lossy(&c).to_ascii_lowercase())
                } else {
                    None
                };
                let prop = |host: &mut dyn Host, p: &str| {
                    host.object_prop(&v, p.as_bytes())
                        .map(|x| conv::to_string_bytes(&deref(&x)))
                        .unwrap_or_default()
                };
                match class.as_deref() {
                    Some("curlfile") | Some("curlstringfile") => {
                        let (data, fname, mime) = if class.as_deref() == Some("curlfile") {
                            let path = prop(host, "name");
                            let post = prop(host, "postname");
                            let data = std::fs::read(String::from_utf8_lossy(&path).as_ref())
                                .unwrap_or_default();
                            let fname = if post.is_empty() {
                                let p = String::from_utf8_lossy(&path).into_owned();
                                p.rsplit('/').next().unwrap_or("").as_bytes().to_vec()
                            } else {
                                post
                            };
                            (data, fname, prop(host, "mime"))
                        } else {
                            (
                                prop(host, "data"),
                                prop(host, "postname"),
                                prop(host, "mime"),
                            )
                        };
                        out.extend_from_slice(
                            format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"")
                                .as_bytes(),
                        );
                        out.extend_from_slice(&fname);
                        out.extend_from_slice(b"\"\r\n");
                        let mime = if mime.is_empty() {
                            b"application/octet-stream".to_vec()
                        } else {
                            mime
                        };
                        out.extend_from_slice(b"Content-Type: ");
                        out.extend_from_slice(&mime);
                        out.extend_from_slice(b"\r\n\r\n");
                        out.extend_from_slice(&data);
                    }
                    _ => {
                        out.extend_from_slice(
                            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n")
                                .as_bytes(),
                        );
                        out.extend_from_slice(&conv::to_string_bytes(&v));
                    }
                }
                out.extend_from_slice(b"\r\n");
            }
            out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
            (out, format!("multipart/form-data; boundary={boundary}"))
        }
        Value::Array(_) => (Vec::new(), "application/x-www-form-urlencoded".into()),
        other => (
            conv::to_string_bytes(other),
            "application/x-www-form-urlencoded".into(),
        ),
    }
}

/// Snapshot a handle's options into a `Send` transfer description.
fn build_spec(host: &mut dyn Host, easy: &Rc<RefCell<Easy>>) -> Spec {
    let e = easy.borrow().clone();
    let (body, ct) = match &e.postfields {
        Some(pf) if e.post && !e.nobody => {
            let (b, ct) = build_body(host, pf);
            (Some(b), Some(ct))
        }
        _ => (None, None),
    };
    let upload = if e.upload {
        e.infile
            .as_ref()
            .and_then(|s| host.stream_read(s, -1))
            .or(Some(Vec::new()))
    } else {
        None
    };
    let userpwd = e
        .userpwd
        .clone()
        .or_else(|| match (&e.username, &e.password) {
            (Some(u), p) => Some(format!("{u}:{}", p.clone().unwrap_or_default())),
            (None, Some(p)) => Some(format!(":{p}")),
            _ => None,
        });
    Spec {
        url: e.url.clone(),
        custom_request: e.custom_request.clone(),
        nobody: e.nobody,
        post: e.post,
        upload,
        body,
        body_content_type: ct,
        headers: e.httpheader.clone(),
        follow: e.follow,
        max_redirs: e.max_redirs,
        timeout_ms: e.timeout_ms,
        connect_timeout_ms: e.connect_timeout_ms,
        proxy: e.proxy.clone(),
        userpwd,
        useragent: e.useragent.clone(),
        referer: e.referer.clone(),
        cookie: e.cookie.clone(),
        range: e.range.clone(),
        encoding: e.encoding.clone(),
        ipresolve: e.ipresolve,
        protocols: e.protocols,
        redir_protocols: e.redir_protocols,
        failonerror: e.failonerror,
        http_version: e.misc.get(&OPT_HTTP_VERSION).map(conv::to_int).unwrap_or(0),
        cainfo: e.cainfo.clone(),
        capath: e.capath.clone(),
        verify_peer: e
            .misc
            .get(&OPT_SSL_VERIFYPEER)
            .is_none_or(|v| conv::to_int(v) != 0),
        unrestricted_auth: e
            .misc
            .get(&OPT_UNRESTRICTED_AUTH)
            .is_some_and(|v| conv::to_int(v) != 0),
        autoreferer: e
            .misc
            .get(&OPT_AUTOREFERER)
            .is_some_and(|v| conv::to_int(v) != 0),
    }
}

/// php's `_php_curl_verify_handlers`: a stream handler whose resource has
/// been closed is dropped (with a warning when `report`), falling back to
/// the default handler.
fn verify_handlers(
    host: &mut dyn Host,
    f: &str,
    easy: &Rc<RefCell<Easy>>,
    report: bool,
) -> Result<(), Thrown> {
    let gone =
        |host: &dyn Host, v: &Option<Value>| v.as_ref().is_some_and(|s| !host.stream_is_open(s));
    let (stderr, infile, header, write) = {
        let e = easy.borrow();
        (
            gone(host, &e.misc.get(&OPT_STDERR).cloned()),
            gone(host, &e.infile),
            gone(host, &e.header_stream),
            gone(host, &e.write_stream),
        )
    };
    for (bad, option, msg) in [
        (
            stderr,
            OPT_STDERR,
            "CURLOPT_STDERR resource has gone away, resetting to stderr",
        ),
        (
            infile,
            OPT_INFILE,
            "CURLOPT_INFILE resource has gone away, resetting to default",
        ),
        (
            header,
            OPT_WRITEHEADER,
            "CURLOPT_WRITEHEADER resource has gone away, resetting to default",
        ),
        (
            write,
            OPT_FILE,
            "CURLOPT_FILE resource has gone away, resetting to default",
        ),
    ] {
        if !bad {
            continue;
        }
        {
            let mut e = easy.borrow_mut();
            match option {
                OPT_STDERR => {
                    e.misc.remove(&OPT_STDERR);
                }
                OPT_INFILE => e.infile = None,
                OPT_WRITEHEADER => {
                    e.header_stream = None;
                    e.header = HeaderMethod::Ignore;
                }
                _ => {
                    e.write_stream = None;
                    e.write = WriteMethod::Stdout;
                }
            }
        }
        if report {
            crate::warn(host, f, msg)?;
        }
    }
    Ok(())
}

thread_local! {
    /// The exception a curl callback threw, held until the curl function that
    /// drove the transfer returns (php leaves it in EG(exception) while
    /// libcurl finishes; zend_call_function then skips every later callback).
    static PENDING: RefCell<Option<Thrown>> = const { RefCell::new(None) };
}

/// Re-throw the exception a callback raised during this call, if any.
fn take_pending() -> Result<(), Thrown> {
    match PENDING.with(|p| p.borrow_mut().take()) {
        Some(t) => Err(t),
        None => Ok(()),
    }
}

/// Call a user callback with the handle flagged as "in callback". `None` is
/// php's undefined retval: the callback threw (or an exception is already
/// pending, so it was not called) and the caller keeps its default result.
fn call_cb(
    host: &mut dyn Host,
    easy: &Rc<RefCell<Easy>>,
    cb: &Value,
    args: Vec<Value>,
) -> Result<Option<Value>, Thrown> {
    if PENDING.with(|p| p.borrow().is_some()) {
        return Ok(None);
    }
    let was = std::mem::replace(&mut easy.borrow_mut().in_callback, true);
    let r = host.call_callable(cb, &args);
    easy.borrow_mut().in_callback = was;
    match r {
        Ok(v) => Ok(Some(v)),
        Err(t) if t.is_catchable() => {
            PENDING.with(|p| *p.borrow_mut() = Some(t));
            Ok(None)
        }
        Err(t) => Err(t),
    }
}

struct Sink {
    /// The php function driving the transfer (for warnings).
    f: &'static str,
    write: WriteMethod,
    write_stream: Option<Value>,
    write_fn: Option<Value>,
    header: HeaderMethod,
    header_stream: Option<Value>,
    ret: Vec<u8>,
}

impl Sink {
    /// After a callback returned: php re-verifies the stream handlers (the
    /// callback may have closed one) and libcurl uses what is left.
    fn after_cb(&mut self, host: &mut dyn Host, easy: &Rc<RefCell<Easy>>) -> Result<(), Thrown> {
        verify_handlers(host, self.f, easy, true)?;
        let e = easy.borrow();
        if self.write == WriteMethod::File && e.write_stream.is_none() {
            self.write = e.write;
            self.write_stream = None;
        }
        if self.header == HeaderMethod::File && e.header_stream.is_none() {
            self.header = e.header;
            self.header_stream = None;
        }
        Ok(())
    }
}

/// Body bytes → the handle's write handler. `Some((passed, returned))` on a
/// short write (libcurl aborts with CURLE_WRITE_ERROR).
fn write_body(
    host: &mut dyn Host,
    easy: &Rc<RefCell<Easy>>,
    obj: &Value,
    sink: &mut Sink,
    data: &[u8],
) -> Result<Option<(usize, i64)>, Thrown> {
    match sink.write {
        WriteMethod::Stdout => host.write_out(data),
        WriteMethod::Return => sink.ret.extend_from_slice(data),
        WriteMethod::File => {
            if let Some(s) = &sink.write_stream {
                let n = host.stream_write(s, data).unwrap_or(0);
                if n != data.len() {
                    return Ok(Some((data.len(), n as i64)));
                }
            }
        }
        WriteMethod::User => {
            if let Some(cb) = sink.write_fn.clone() {
                let r = call_cb(
                    host,
                    easy,
                    &cb,
                    vec![obj.clone(), Value::str(data.to_vec())],
                )?;
                let n = match r {
                    Some(r) => {
                        sink.after_cb(host, easy)?;
                        conv::to_int(&deref(&r))
                    }
                    None => data.len() as i64,
                };
                if n != data.len() as i64 {
                    return Ok(Some((data.len(), n)));
                }
            }
        }
    }
    Ok(None)
}

/// Hand a finished transfer's data to PHP (header handler, body handler,
/// progress callbacks) and record its result on the handle. Returns the
/// final CURLcode.
fn deliver(
    host: &mut dyn Host,
    f: &'static str,
    obj: &Value,
    easy: &Rc<RefCell<Easy>>,
    out: Outcome,
) -> Result<i64, Thrown> {
    let e = easy.borrow().clone();
    let mut sink = Sink {
        f,
        write: e.write,
        write_stream: e.write_stream.clone(),
        write_fn: e.write_fn.clone(),
        header: e.header,
        header_stream: e.header_stream.clone(),
        ret: Vec::new(),
    };
    let mut errno = out.errno;
    let mut error = out.error;
    let mut aborted = false;

    'hdr: for block in &out.header_blocks {
        for line in block {
            match sink.header {
                HeaderMethod::Ignore => {}
                HeaderMethod::File => {
                    if let Some(s) = &sink.header_stream {
                        host.stream_write(s, line);
                    }
                }
                HeaderMethod::User => {
                    if let Some(cb) = &e.header_fn {
                        let r =
                            call_cb(host, easy, cb, vec![obj.clone(), Value::str(line.clone())])?;
                        let n = match r {
                            Some(r) => {
                                sink.after_cb(host, easy)?;
                                conv::to_int(&deref(&r))
                            }
                            None => line.len() as i64,
                        };
                        if n != line.len() as i64 {
                            errno = 23;
                            error = format!(
                                "Failure writing output to destination, passed {} returned {}",
                                line.len(),
                                n
                            );
                            aborted = true;
                            break 'hdr;
                        }
                    }
                }
            }
            if e.include_header {
                if let Some((p, r)) = write_body(host, easy, obj, &mut sink, line)? {
                    errno = 23;
                    error =
                        format!("Failure writing output to destination, passed {p} returned {r}");
                    aborted = true;
                    break 'hdr;
                }
            }
        }
    }

    let total = if out.info.content_length_download >= 0.0 {
        out.info.content_length_download as i64
    } else {
        0
    };
    let progress_cb = if e.noprogress {
        None
    } else if let Some(x) = &e.xferinfo_fn {
        Some(x.clone())
    } else {
        e.progress_fn.clone()
    };
    if !aborted {
        if let Some(cb) = &progress_cb {
            let r = call_cb(
                host,
                easy,
                cb,
                vec![
                    obj.clone(),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                    Value::Int(0),
                ],
            )?;
            if r.is_some() {
                sink.after_cb(host, easy)?;
            }
            if r.is_some_and(|r| conv::to_int(&deref(&r)) != 0) {
                errno = 42;
                error = "Callback aborted".into();
                aborted = true;
            }
        }
    }
    if !aborted {
        for chunk in out.body.chunks(16384) {
            if let Some((p, r)) = write_body(host, easy, obj, &mut sink, chunk)? {
                errno = 23;
                error = format!("Failure writing output to destination, passed {p} returned {r}");
                aborted = true;
                break;
            }
        }
    }
    if !aborted && errno == 0 {
        if let Some(cb) = &progress_cb {
            let now = out.info.size_download;
            let r = call_cb(
                host,
                easy,
                cb,
                vec![
                    obj.clone(),
                    Value::Int(total.max(now)),
                    Value::Int(now),
                    Value::Int(0),
                    Value::Int(0),
                ],
            )?;
            if r.is_some() {
                sink.after_cb(host, easy)?;
            }
            if r.is_some_and(|r| conv::to_int(&deref(&r)) != 0) {
                errno = 42;
                error = "Callback aborted".into();
            }
        }
    }

    let mut eb = easy.borrow_mut();
    eb.info = Some(out.info);
    eb.errno = errno;
    eb.error = error;
    eb.ret_buf = if e.write == WriteMethod::Return {
        Some(sink.ret)
    } else {
        None
    };
    Ok(errno)
}

fn fn_curl_exec(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (h, easy) = easy_arg(host, "curl_exec", 1, &args[0])?;
    verify_handlers(host, "curl_exec", &easy, true)?;
    easy.borrow_mut().cleanup();
    let spec = build_spec(host, &easy);
    let out = net::perform(&spec);
    let obj = Value::Object(h);
    let errno = deliver(host, "curl_exec", &obj, &easy, out)?;
    take_pending()?;
    let mut e = easy.borrow_mut();
    if errno != 0 {
        e.ret_buf = None;
        return Ok(Value::Bool(false));
    }
    if e.write == WriteMethod::Return {
        let buf = e.ret_buf.clone().unwrap_or_default();
        return Ok(Value::str(buf));
    }
    Ok(Value::Bool(true))
}

// ---------------------------------------------------------------------------
// getinfo / errors / misc easy functions
// ---------------------------------------------------------------------------

const T_STRING: i64 = 0x100000;
const T_LONG: i64 = 0x200000;
const T_DOUBLE: i64 = 0x300000;
const T_SLIST: i64 = 0x400000;
const T_OFF_T: i64 = 0x600000;

fn secs(us: i64) -> f64 {
    us as f64 / 1_000_000.0
}

fn speed(bytes: i64, total_us: i64) -> i64 {
    if total_us > 0 {
        (bytes as f64 * 1_000_000.0 / total_us as f64) as i64
    } else {
        0
    }
}

fn opt_s(s: &Option<String>) -> Value {
    match s {
        Some(s) => Value::str(s.clone()),
        None => Value::Null,
    }
}

/// One CURLINFO value, or `None` when libcurl would fail the query.
fn info_value(i: &Info, option: i64) -> Option<Value> {
    let ty = option & 0xf00000;
    let n = option & 0xfffff;
    let v = match (ty, n) {
        (T_STRING, 1) => Value::str(i.effective_url.clone()),
        (T_LONG, 2) => Value::Int(i.http_code),
        (T_DOUBLE, 3) => Value::Float(secs(i.total_us)),
        (T_DOUBLE, 4) => Value::Float(secs(i.namelookup_us)),
        (T_DOUBLE, 5) => Value::Float(secs(i.connect_us)),
        (T_DOUBLE, 6) => Value::Float(secs(i.pretransfer_us)),
        (T_DOUBLE, 7) => Value::Float(i.size_upload as f64),
        (T_OFF_T, 7) => Value::Int(i.size_upload),
        (T_DOUBLE, 8) => Value::Float(i.size_download as f64),
        (T_OFF_T, 8) => Value::Int(i.size_download),
        (T_DOUBLE, 9) => Value::Float(speed(i.size_download, i.total_us) as f64),
        (T_OFF_T, 9) => Value::Int(speed(i.size_download, i.total_us)),
        (T_DOUBLE, 10) => Value::Float(speed(i.size_upload, i.total_us) as f64),
        (T_OFF_T, 10) => Value::Int(speed(i.size_upload, i.total_us)),
        (T_LONG, 11) => Value::Int(i.header_size),
        (T_LONG, 12) => Value::Int(i.request_size),
        (T_LONG, 13) => Value::Int(0),
        (T_LONG, 14) | (T_OFF_T, 14) => Value::Int(i.filetime),
        (T_DOUBLE, 15) => Value::Float(i.content_length_download),
        (T_OFF_T, 15) => Value::Int(i.content_length_download as i64),
        (T_DOUBLE, 16) => Value::Float(i.content_length_upload),
        (T_OFF_T, 16) => Value::Int(i.content_length_upload as i64),
        (T_DOUBLE, 17) => Value::Float(secs(i.starttransfer_us)),
        (T_STRING, 18) => opt_s(&i.content_type),
        (T_DOUBLE, 19) => Value::Float(secs(i.redirect_us)),
        (T_LONG, 20) => Value::Int(i.redirect_count),
        (T_LONG, 22) | (T_LONG, 23) | (T_LONG, 24) | (T_LONG, 25) => Value::Int(0),
        (T_LONG, 26) => Value::Int((i.conn_id >= 0) as i64),
        (T_SLIST, 27) | (T_SLIST, 28) => Value::Array(PhpArray::new()),
        (T_LONG, 29) => Value::Int(-1),
        (T_STRING, 30) => Value::Null,
        (T_STRING, 31) => opt_s(&i.redirect_url),
        (T_STRING, 32) => Value::str(i.primary_ip.clone()),
        (T_DOUBLE, 33) => Value::Float(secs(i.appconnect_us)),
        (T_LONG, 35) => Value::Int(0),
        (T_LONG, 40) => Value::Int(i.primary_port),
        (T_STRING, 41) => Value::str(i.local_ip.clone()),
        (T_LONG, 42) => Value::Int(i.local_port),
        (T_LONG, 46) => Value::Int(i.http_version),
        (T_LONG, 47) => Value::Int(0),
        (T_LONG, 48) => Value::Int(i.protocol),
        (T_STRING, 49) => opt_s(&i.scheme),
        (T_OFF_T, 50) => Value::Int(i.total_us),
        (T_OFF_T, 51) => Value::Int(i.namelookup_us),
        (T_OFF_T, 52) => Value::Int(i.connect_us),
        (T_OFF_T, 53) => Value::Int(i.pretransfer_us),
        (T_OFF_T, 54) => Value::Int(i.starttransfer_us),
        (T_OFF_T, 55) => Value::Int(i.redirect_us),
        (T_OFF_T, 56) => Value::Int(i.appconnect_us),
        (T_OFF_T, 57) => Value::Int(0),
        (T_STRING, 58) => Value::str(i.effective_method.clone()),
        (T_LONG, 59) => Value::Int(0),
        (T_STRING, 60) => opt_s(&i.referer),
        (T_STRING, 61) => opt_s(&i.cainfo),
        (T_STRING, 62) => opt_s(&i.capath),
        (T_OFF_T, 63) => Value::Int(i.conn_id),
        (T_OFF_T, 64) => Value::Int(i.conn_id),
        (T_OFF_T, 65) => Value::Int(i.queue_us),
        (T_LONG, 66) => Value::Int(i.used_proxy),
        (T_OFF_T, 67) => Value::Int(i.posttransfer_us),
        (T_OFF_T, 68) => Value::Int(0),
        (T_LONG, 69) | (T_LONG, 70) => Value::Int(0),
        _ => return None,
    };
    Some(v)
}

fn info_array(i: &Info) -> PhpArray {
    let mut a = PhpArray::new();
    let s = |o: &Option<String>| Value::str(o.clone().unwrap_or_default());
    let mut put = |k: &str, v: Value| a.insert(akey(k), v);
    put("url", Value::str(i.effective_url.clone()));
    put("content_type", opt_s(&i.content_type));
    put("http_code", Value::Int(i.http_code));
    put("header_size", Value::Int(i.header_size));
    put("request_size", Value::Int(i.request_size));
    put("filetime", Value::Int(i.filetime));
    put("ssl_verify_result", Value::Int(0));
    put("redirect_count", Value::Int(i.redirect_count));
    put("total_time", Value::Float(secs(i.total_us)));
    put("namelookup_time", Value::Float(secs(i.namelookup_us)));
    put("connect_time", Value::Float(secs(i.connect_us)));
    put("pretransfer_time", Value::Float(secs(i.pretransfer_us)));
    put("size_upload", Value::Float(i.size_upload as f64));
    put("size_download", Value::Float(i.size_download as f64));
    put(
        "speed_download",
        Value::Float(speed(i.size_download, i.total_us) as f64),
    );
    put(
        "speed_upload",
        Value::Float(speed(i.size_upload, i.total_us) as f64),
    );
    put(
        "download_content_length",
        Value::Float(i.content_length_download),
    );
    put(
        "upload_content_length",
        Value::Float(i.content_length_upload),
    );
    put("starttransfer_time", Value::Float(secs(i.starttransfer_us)));
    put("redirect_time", Value::Float(secs(i.redirect_us)));
    put("redirect_url", s(&i.redirect_url));
    put("primary_ip", Value::str(i.primary_ip.clone()));
    put("certinfo", Value::Array(PhpArray::new()));
    put("primary_port", Value::Int(i.primary_port));
    put("local_ip", Value::str(i.local_ip.clone()));
    put("local_port", Value::Int(i.local_port));
    put("http_version", Value::Int(i.http_version));
    put("protocol", Value::Int(i.protocol));
    put("ssl_verifyresult", Value::Int(0));
    put("scheme", s(&i.scheme));
    put("appconnect_time_us", Value::Int(i.appconnect_us));
    put("queue_time_us", Value::Int(i.queue_us));
    put("connect_time_us", Value::Int(i.connect_us));
    put("namelookup_time_us", Value::Int(i.namelookup_us));
    put("pretransfer_time_us", Value::Int(i.pretransfer_us));
    put("redirect_time_us", Value::Int(i.redirect_us));
    put("starttransfer_time_us", Value::Int(i.starttransfer_us));
    put("posttransfer_time_us", Value::Int(i.posttransfer_us));
    put("total_time_us", Value::Int(i.total_us));
    put("effective_method", Value::str(i.effective_method.clone()));
    put("capath", s(&i.capath));
    put("cainfo", s(&i.cainfo));
    put("used_proxy", Value::Int(i.used_proxy));
    put("httpauth_used", Value::Int(0));
    put("proxyauth_used", Value::Int(0));
    put("conn_id", Value::Int(i.conn_id));
    a
}

fn current_info(e: &Easy) -> Info {
    match &e.info {
        Some(i) => i.clone(),
        None => Info::fresh(&e.url, &e.method()),
    }
}

fn fn_curl_getinfo(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, easy) = easy_arg(host, "curl_getinfo", 1, &args[0])?;
    let option = match args.get(1).map(deref) {
        None | Some(Value::Null) => None,
        Some(v) => Some(arg_long(host, "curl_getinfo", 2, "option", &v)?),
    };
    let e = easy.borrow();
    let info = current_info(&e);
    let Some(option) = option else {
        return Ok(Value::Array(info_array(&info)));
    };
    match option {
        INFO_HEADER_OUT => Ok(Value::Bool(false)),
        INFO_CERTINFO => Ok(Value::Array(PhpArray::new())),
        INFO_PRIVATE => Ok(e.private.clone().unwrap_or(Value::Bool(false))),
        _ => Ok(match info_value(&info, option) {
            Some(Value::Null) | None => Value::Bool(false),
            Some(v) => v,
        }),
    }
}

fn fn_curl_error(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, easy) = easy_arg(host, "curl_error", 1, &args[0])?;
    let e = easy.borrow();
    Ok(Value::str(if e.errno == 0 {
        String::new()
    } else if !e.error.is_empty() {
        e.error.clone()
    } else {
        easy_strerror(e.errno).to_string()
    }))
}

fn fn_curl_errno(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, easy) = easy_arg(host, "curl_errno", 1, &args[0])?;
    let n = easy.borrow().errno;
    Ok(Value::Int(n))
}

fn fn_curl_strerror(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let code = arg_long(host, "curl_strerror", 1, "error_code", &args[0])?;
    Ok(Value::str(easy_strerror(code)))
}

fn fn_curl_close(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    crate::deprecated_fn(
        host,
        "curl_close",
        "8.5",
        "as it has no effect since PHP 8.0",
    )?;
    let (_, easy) = easy_arg(host, "curl_close", 1, &args[0])?;
    if easy.borrow().in_callback {
        return Err(Thrown::error(
            "Error",
            "curl_close(): Attempt to close cURL handle from a callback",
        ));
    }
    Ok(Value::Null)
}

fn fn_curl_reset(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, easy) = easy_arg(host, "curl_reset", 1, &args[0])?;
    let mut e = easy.borrow_mut();
    if e.in_callback {
        return Err(Thrown::error(
            "Error",
            "curl_reset(): Attempt to reset cURL handle from a callback",
        ));
    }
    // curl_easy_reset + _php_curl_reset_handlers: CURLOPT_PRIVATE and the
    // last error (ch->err) are php-side state that survive.
    let fresh = Easy {
        multi: e.multi,
        private: e.private.take(),
        errno: e.errno,
        error: std::mem::take(&mut e.error),
        ..Easy::new()
    };
    *e = fresh;
    Ok(Value::Null)
}

/// The state of a duplicated easy handle (curl_easy_duphandle plus php's
/// handler copies): options and callbacks carry over, transfer state does not.
fn dup_easy(easy: &Rc<RefCell<Easy>>) -> Easy {
    let mut copy = easy.borrow().clone();
    copy.multi = MultiState::Idle;
    copy.in_callback = false;
    copy.info = None;
    copy.cleanup();
    copy
}

fn fn_curl_copy_handle(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, easy) = easy_arg(host, "curl_copy_handle", 1, &args[0])?;
    let copy = dup_easy(&easy);
    register_easy(host, copy)
}

/// `clone` of a curl object, part one (before the copy exists): the class
/// name when it is a CurlMultiHandle / CurlShareHandle, whose clone_obj
/// handler is NULL ("Trying to clone an uncloneable object of class ...").
pub fn curl_uncloneable(h: &ObjectHandle) -> Option<&'static str> {
    REG.with(|r| {
        let r = r.borrow();
        if lookup(&r.multi, h).is_some() {
            Some("CurlMultiHandle")
        } else if lookup(&r.share, h).is_some() {
            Some("CurlShareHandle")
        } else {
            None
        }
    })
}

/// `clone` of a curl object, part two: php's curl_clone_obj gives the clone
/// its own copy of the easy handle, as curl_copy_handle() does.
pub fn curl_cloned(src: &ObjectHandle, dst: &ObjectHandle) {
    let Some(easy) = REG.with(|r| lookup(&r.borrow().easy, src)) else {
        return;
    };
    let copy = dup_easy(&easy);
    REG.with(|r| {
        let mut r = r.borrow_mut();
        r.sweep();
        r.easy.insert(
            key_of(dst),
            Entry {
                obj: Rc::downgrade(dst),
                data: Rc::new(RefCell::new(copy)),
            },
        );
    });
}

fn arg_str(host: &dyn Host, f: &str, n: usize, pname: &str, v: &Value) -> Result<Vec<u8>, Thrown> {
    match deref(v) {
        Value::Array(_) | Value::Object(_) | Value::Closure(_) | Value::Resource(_) => {
            Err(Thrown::type_error(format!(
                "{f}(): Argument #{n} (${pname}) must be of type string, {} given",
                value_name(host, &deref(v))
            )))
        }
        o => Ok(conv::to_string_bytes(&o)),
    }
}

fn fn_curl_escape(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    easy_arg(host, "curl_escape", 1, &args[0])?;
    let s = arg_str(host, "curl_escape", 2, "string", &args[1])?;
    let mut out = String::new();
    for b in s {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    Ok(Value::str(out))
}

fn fn_curl_unescape(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    easy_arg(host, "curl_unescape", 1, &args[0])?;
    let s = arg_str(host, "curl_unescape", 2, "string", &args[1])?;
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'%' && i + 2 < s.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(s[i + 1]), hex(s[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(s[i]);
        i += 1;
    }
    Ok(Value::str(out))
}

fn fn_curl_pause(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    easy_arg(host, "curl_pause", 1, &args[0])?;
    arg_long(host, "curl_pause", 2, "flags", &args[1])?;
    Ok(Value::Int(0))
}

fn fn_curl_upkeep(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    easy_arg(host, "curl_upkeep", 1, &args[0])?;
    Ok(Value::Bool(true))
}

fn fn_curl_version(_: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut a = PhpArray::new();
    let features: i64 = 1472153485;
    a.insert(akey("version_number"), Value::Int(529664));
    a.insert(akey("age"), Value::Int(11));
    a.insert(akey("features"), Value::Int(features));
    let feats: &[(&str, i64)] = &[
        ("AsynchDNS", 1 << 7),
        ("CharConv", 1 << 12),
        ("Debug", 1 << 6),
        ("GSS-Negotiate", 1 << 5),
        ("IDN", 1 << 10),
        ("IPv6", 1),
        ("krb4", 1 << 1),
        ("Largefile", 1 << 9),
        ("libz", 1 << 3),
        ("NTLM", 1 << 4),
        ("NTLMWB", 1 << 15),
        ("SPNEGO", 1 << 8),
        ("SSL", 1 << 2),
        ("SSPI", 1 << 11),
        ("TLS-SRP", 1 << 14),
        ("HTTP2", 1 << 16),
        ("GSSAPI", 1 << 17),
        ("KERBEROS5", 1 << 18),
        ("UNIX_SOCKETS", 1 << 19),
        ("PSL", 1 << 20),
        ("HTTPS_PROXY", 1 << 21),
        ("MULTI_SSL", 1 << 22),
        ("BROTLI", 1 << 23),
        ("ALTSVC", 1 << 24),
        ("HTTP3", 1 << 25),
        ("UNICODE", 1 << 27),
        ("ZSTD", 1 << 26),
        ("HSTS", 1 << 28),
        ("GSASL", 1 << 29),
    ];
    let mut fl = PhpArray::new();
    for (n, bit) in feats {
        fl.insert(akey(n), Value::Bool(features & bit != 0));
    }
    a.insert(akey("feature_list"), Value::Array(fl));
    a.insert(akey("ssl_version_number"), Value::Int(0));
    a.insert(akey("version"), Value::str("8.21.0"));
    a.insert(akey("host"), Value::str("aarch64-apple-darwin25.4.0"));
    a.insert(akey("ssl_version"), Value::str("OpenSSL/3.6.4"));
    a.insert(akey("libz_version"), Value::str("1.2.12"));
    let mut protos = PhpArray::new();
    for p in [
        "dict", "file", "ftp", "ftps", "gopher", "gophers", "http", "https", "imap", "imaps",
        "ldap", "ldaps", "mqtt", "mqtts", "pop3", "pop3s", "rtsp", "scp", "sftp", "smtp", "smtps",
        "telnet", "tftp", "ws", "wss",
    ] {
        protos.push(Value::str(p));
    }
    a.insert(akey("protocols"), Value::Array(protos));
    a.insert(akey("ares"), Value::str(""));
    a.insert(akey("ares_num"), Value::Int(0));
    a.insert(akey("libidn"), Value::str(""));
    a.insert(akey("iconv_ver_num"), Value::Int(0));
    a.insert(akey("libssh_version"), Value::str("libssh2/1.11.1"));
    a.insert(akey("brotli_ver_num"), Value::Int(16785408));
    a.insert(akey("brotli_version"), Value::str("brotli/1.2.0"));
    Ok(Value::Array(a))
}

fn fn_curl_file_create(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = arg_str(host, "curl_file_create", 1, "filename", &args[0])?;
    let opt = |i: usize| -> Result<Vec<u8>, Thrown> {
        match args.get(i).map(deref) {
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(v) => arg_str(
                host,
                "curl_file_create",
                i + 1,
                if i == 1 {
                    "mime_type"
                } else {
                    "posted_filename"
                },
                &v,
            ),
        }
    };
    let mime = opt(1)?;
    let post = opt(2)?;
    host.instantiate(
        b"CURLFile",
        vec![
            (b"name".to_vec(), Value::str(name)),
            (b"mime".to_vec(), Value::str(mime)),
            (b"postname".to_vec(), Value::str(post)),
        ],
    )
    .ok_or_else(|| Thrown::unsupported("curl", "cannot instantiate CURLFile"))
}

// ---------------------------------------------------------------------------
// multi
// ---------------------------------------------------------------------------

fn fn_curl_multi_init(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let (v, h) = new_object(host, "CurlMultiHandle")?;
    REG.with(|r| {
        let mut r = r.borrow_mut();
        r.sweep();
        r.multi.insert(
            key_of(&h),
            Entry {
                obj: Rc::downgrade(&h),
                data: Rc::new(RefCell::new(Multi::new())),
            },
        );
    });
    Ok(v)
}

fn same_obj(a: &Value, b: &ObjectHandle) -> bool {
    obj_handle(a).map(|h| Rc::ptr_eq(&h, b)).unwrap_or(false)
}

fn fn_curl_multi_add_handle(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, multi) = multi_arg(host, "curl_multi_add_handle", &args[0])?;
    let (eh, easy) = easy_arg(host, "curl_multi_add_handle", 2, &args[1])?;
    verify_handlers(host, "curl_multi_add_handle", &easy, true)?;
    let mut e = easy.borrow_mut();
    let code = if e.multi != MultiState::Idle {
        7
    } else {
        e.cleanup();
        e.multi = MultiState::Pending;
        multi.borrow_mut().handles.push(Value::Object(eh));
        0
    };
    drop(e);
    multi.borrow_mut().errno = code;
    Ok(Value::Int(code))
}

fn fn_curl_multi_remove_handle(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, multi) = multi_arg(host, "curl_multi_remove_handle", &args[0])?;
    let (eh, easy) = easy_arg(host, "curl_multi_remove_handle", 2, &args[1])?;
    let mut m = multi.borrow_mut();
    let before = m.handles.len();
    m.handles.retain(|v| !same_obj(v, &eh));
    if m.handles.len() != before {
        let mut e = easy.borrow_mut();
        if let MultiState::Running(t) = e.multi {
            m.running.remove(&t);
        }
        e.multi = MultiState::Idle;
    }
    m.errno = 0;
    Ok(Value::Int(0))
}

/// Start every pending transfer on a worker thread.
fn start_pending(host: &mut dyn Host, multi: &Rc<RefCell<Multi>>) {
    let handles = multi.borrow().handles.clone();
    for hv in handles {
        let Some(h) = obj_handle(&hv) else { continue };
        let Some(easy) = REG.with(|r| lookup(&r.borrow().easy, &h)) else {
            continue;
        };
        if easy.borrow().multi != MultiState::Pending {
            continue;
        }
        let spec = build_spec(host, &easy);
        let mut m = multi.borrow_mut();
        let ticket = m.next_ticket;
        m.next_ticket += 1;
        m.running.insert(ticket, hv.clone());
        easy.borrow_mut().multi = MultiState::Running(ticket);
        let tx = m.tx.clone();
        drop(m);
        let spawned = std::thread::Builder::new()
            .name("rhp-curl".into())
            .spawn(move || {
                let out = net::perform(&spec);
                let _ = tx.send((ticket, out));
            });
        if spawned.is_err() {
            // No thread available: run it inline.
            let out = net::perform(&build_spec(host, &easy));
            multi.borrow_mut().ready.push((ticket, out));
        }
    }
}

fn fn_curl_multi_exec(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, multi) = multi_arg(host, "curl_multi_exec", &args[0])?;
    let handles = multi.borrow().handles.clone();
    for hv in &handles {
        if let Some(easy) = obj_handle(hv).and_then(|h| REG.with(|r| lookup(&r.borrow().easy, &h)))
        {
            verify_handlers(host, "curl_multi_exec", &easy, true)?;
        }
    }
    start_pending(host, &multi);
    // Collect finished transfers.
    let done: Vec<(Value, Outcome)> = {
        let mut m = multi.borrow_mut();
        let mut ready: Vec<(u64, Outcome)> = std::mem::take(&mut m.ready);
        ready.extend(m.rx.try_iter());
        ready
            .into_iter()
            .filter_map(|(t, o)| m.running.remove(&t).map(|h| (h, o)))
            .collect()
    };
    for (hv, out) in done {
        let Some(h) = obj_handle(&hv) else { continue };
        let Some(easy) = REG.with(|r| lookup(&r.borrow().easy, &h)) else {
            continue;
        };
        easy.borrow_mut().multi = MultiState::Idle;
        // A callback's exception is held (see call_cb) so every finished
        // transfer is still delivered and reported before it is re-thrown.
        let code = deliver(host, "curl_multi_exec", &hv, &easy, out)?;
        // Still part of the multi (php keeps it until remove_handle).
        easy.borrow_mut().multi = if multi.borrow().handles.iter().any(|v| same_obj(v, &h)) {
            MultiState::Running(0)
        } else {
            MultiState::Idle
        };
        multi.borrow_mut().msgs.push_back((code, hv));
    }
    let running = multi.borrow().running.len() as i64;
    if let Some(slot) = args.get(1) {
        set_ref(slot, Value::Int(running));
    }
    multi.borrow_mut().errno = 0;
    take_pending()?;
    Ok(Value::Int(0))
}

fn fn_curl_multi_select(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, multi) = multi_arg(host, "curl_multi_select", &args[0])?;
    let timeout = match args.get(1) {
        Some(v) => arg_float(host, "curl_multi_select", 2, "timeout", v)?,
        None => 1.0,
    };
    let max = i32::MAX as f64 / 1000.0;
    if !(timeout >= 0.0 && timeout <= max) {
        return Err(Thrown::error(
            "ValueError",
            format!("curl_multi_select(): Argument #2 ($timeout) must be between 0 and {max:.6}"),
        ));
    }
    let mut m = multi.borrow_mut();
    if !m.ready.is_empty() {
        return Ok(Value::Int(m.ready.len() as i64));
    }
    if m.running.is_empty() {
        return Ok(Value::Int(0));
    }
    match m.rx.recv_timeout(Duration::from_secs_f64(timeout)) {
        Ok(first) => {
            m.ready.push(first);
            let more: Vec<_> = m.rx.try_iter().collect();
            m.ready.extend(more);
            Ok(Value::Int(m.ready.len() as i64))
        }
        Err(_) => Ok(Value::Int(0)),
    }
}

fn fn_curl_multi_info_read(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, multi) = multi_arg(host, "curl_multi_info_read", &args[0])?;
    let mut m = multi.borrow_mut();
    let Some((code, hv)) = m.msgs.pop_front() else {
        return Ok(Value::Bool(false));
    };
    if let Some(slot) = args.get(1) {
        set_ref(slot, Value::Int(m.msgs.len() as i64));
    }
    drop(m);
    let mut a = PhpArray::new();
    a.insert(akey("msg"), Value::Int(1));
    a.insert(akey("result"), Value::Int(code));
    if let Some(h) = obj_handle(&hv) {
        if let Some(easy) = REG.with(|r| lookup(&r.borrow().easy, &h)) {
            let mut e = easy.borrow_mut();
            e.errno = code;
            if code == 0 {
                e.error.clear();
            }
        }
    }
    a.insert(akey("handle"), hv);
    Ok(Value::Array(a))
}

fn fn_curl_multi_getcontent(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, easy) = easy_arg(host, "curl_multi_getcontent", 1, &args[0])?;
    let e = easy.borrow();
    if e.write == WriteMethod::Return {
        return Ok(Value::str(e.ret_buf.clone().unwrap_or_default()));
    }
    Ok(Value::Null)
}

fn fn_curl_multi_get_handles(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, multi) = multi_arg(host, "curl_multi_get_handles", &args[0])?;
    let mut a = PhpArray::new();
    for h in multi.borrow().handles.iter() {
        a.push(h.clone());
    }
    Ok(Value::Array(a))
}

fn fn_curl_multi_close(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, multi) = multi_arg(host, "curl_multi_close", &args[0])?;
    let handles = std::mem::take(&mut multi.borrow_mut().handles);
    for hv in handles {
        if let Some(h) = obj_handle(&hv) {
            if let Some(easy) = REG.with(|r| lookup(&r.borrow().easy, &h)) {
                verify_handlers(host, "curl_multi_close", &easy, true)?;
                easy.borrow_mut().multi = MultiState::Idle;
            }
        }
    }
    multi.borrow_mut().running.clear();
    Ok(Value::Null)
}

fn fn_curl_multi_errno(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, multi) = multi_arg(host, "curl_multi_errno", &args[0])?;
    let n = multi.borrow().errno;
    Ok(Value::Int(n))
}

fn fn_curl_multi_strerror(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let code = arg_long(host, "curl_multi_strerror", 1, "error_code", &args[0])?;
    Ok(Value::str(multi_strerror(code)))
}

fn fn_curl_multi_setopt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let (_, multi) = multi_arg(host, "curl_multi_setopt", &args[0])?;
    let option = arg_long(host, "curl_multi_setopt", 2, "option", &args[1])?;
    let value = deref(&args[2]);
    match option {
        3 | 6 | 30010 | 30009 | 7 | 8 | 13 | 16 => {
            multi.borrow_mut().errno = 0;
            Ok(Value::Bool(true))
        }
        20014 => {
            if !host.is_callable_value(&value) {
                return Err(Thrown::type_error(format!(
                    "curl_multi_setopt(): Argument #2 ($option) must be a valid callback for option CURLMOPT_PUSHFUNCTION, {}",
                    callable_error(&value)
                )));
            }
            Ok(Value::Bool(true))
        }
        _ => {
            multi.borrow_mut().errno = 6;
            Err(Thrown::error(
                "ValueError",
                "curl_multi_setopt(): Argument #2 ($option) is not a valid cURL multi option",
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// share
// ---------------------------------------------------------------------------

fn fn_curl_share_init(host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let (v, h) = new_object(host, "CurlShareHandle")?;
    REG.with(|r| {
        let mut r = r.borrow_mut();
        r.sweep();
        r.share.insert(
            key_of(&h),
            Entry {
                obj: Rc::downgrade(&h),
                data: Rc::new(RefCell::new(Share::default())),
            },
        );
    });
    Ok(v)
}

fn fn_curl_share_setopt(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let share = share_arg(host, "curl_share_setopt", &args[0])?;
    let option = arg_long(host, "curl_share_setopt", 2, "option", &args[1])?;
    match option {
        1 | 2 => {
            let v = conv::to_int(&deref(&args[2]));
            // CURL_LOCK_DATA_SHARE (1) is not a lockable data kind.
            let ok = (2..=7).contains(&v);
            share.borrow_mut().errno = if ok { 0 } else { 1 };
            Ok(Value::Bool(ok))
        }
        _ => {
            share.borrow_mut().errno = 1;
            Err(Thrown::error(
                "ValueError",
                "curl_share_setopt(): Argument #2 ($option) is not a valid cURL share option",
            ))
        }
    }
}

fn fn_curl_share_close(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    crate::deprecated_fn(
        host,
        "curl_share_close",
        "8.5",
        "as it has no effect since PHP 8.0",
    )?;
    share_arg(host, "curl_share_close", &args[0])?;
    Ok(Value::Null)
}

fn fn_curl_share_errno(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let share = share_arg(host, "curl_share_errno", &args[0])?;
    let n = share.borrow().errno;
    Ok(Value::Int(n))
}

fn fn_curl_share_strerror(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let code = arg_long(host, "curl_share_strerror", 1, "error_code", &args[0])?;
    Ok(Value::str(share_strerror(code)))
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

pub fn constants() -> Vec<(&'static str, Value)> {
    consts::CONSTANTS
        .iter()
        .map(|(n, v)| (*n, Value::Int(*v)))
        .collect()
}

pub fn register(m: &mut HashMap<&'static str, Builtin>) {
    fn put(
        m: &mut HashMap<&'static str, Builtin>,
        name: &'static str,
        func: crate::NativeFn,
        min: usize,
        max: Option<usize>,
    ) {
        m.insert(
            name,
            Builtin {
                name,
                func,
                min_args: min,
                max_args: max,
            },
        );
    }
    put(m, "curl_init", fn_curl_init, 0, Some(1));
    put(m, "curl_setopt", fn_curl_setopt, 3, Some(3));
    put(m, "curl_setopt_array", fn_curl_setopt_array, 2, Some(2));
    put(m, "curl_exec", fn_curl_exec, 1, Some(1));
    put(m, "curl_getinfo", fn_curl_getinfo, 1, Some(2));
    put(m, "curl_error", fn_curl_error, 1, Some(1));
    put(m, "curl_errno", fn_curl_errno, 1, Some(1));
    put(m, "curl_strerror", fn_curl_strerror, 1, Some(1));
    put(m, "curl_close", fn_curl_close, 1, Some(1));
    put(m, "curl_reset", fn_curl_reset, 1, Some(1));
    put(m, "curl_copy_handle", fn_curl_copy_handle, 1, Some(1));
    put(m, "curl_escape", fn_curl_escape, 2, Some(2));
    put(m, "curl_unescape", fn_curl_unescape, 2, Some(2));
    put(m, "curl_pause", fn_curl_pause, 2, Some(2));
    put(m, "curl_upkeep", fn_curl_upkeep, 1, Some(1));
    put(m, "curl_version", fn_curl_version, 0, Some(0));
    put(m, "curl_file_create", fn_curl_file_create, 1, Some(3));
    put(m, "curl_multi_init", fn_curl_multi_init, 0, Some(0));
    put(
        m,
        "curl_multi_add_handle",
        fn_curl_multi_add_handle,
        2,
        Some(2),
    );
    put(
        m,
        "curl_multi_remove_handle",
        fn_curl_multi_remove_handle,
        2,
        Some(2),
    );
    put(m, "curl_multi_exec", fn_curl_multi_exec, 2, Some(2));
    put(m, "curl_multi_select", fn_curl_multi_select, 1, Some(2));
    put(
        m,
        "curl_multi_info_read",
        fn_curl_multi_info_read,
        1,
        Some(2),
    );
    put(
        m,
        "curl_multi_getcontent",
        fn_curl_multi_getcontent,
        1,
        Some(1),
    );
    put(
        m,
        "curl_multi_get_handles",
        fn_curl_multi_get_handles,
        1,
        Some(1),
    );
    put(m, "curl_multi_close", fn_curl_multi_close, 1, Some(1));
    put(m, "curl_multi_errno", fn_curl_multi_errno, 1, Some(1));
    put(m, "curl_multi_strerror", fn_curl_multi_strerror, 1, Some(1));
    put(m, "curl_multi_setopt", fn_curl_multi_setopt, 3, Some(3));
    put(m, "curl_share_init", fn_curl_share_init, 0, Some(0));
    put(m, "curl_share_setopt", fn_curl_share_setopt, 3, Some(3));
    put(m, "curl_share_close", fn_curl_share_close, 1, Some(1));
    put(m, "curl_share_errno", fn_curl_share_errno, 1, Some(1));
    put(m, "curl_share_strerror", fn_curl_share_strerror, 1, Some(1));
}

pub const PRELUDE_PHP: &str = r#"
final class CurlHandle {
    public function __construct() { throw new Error("Cannot directly construct CurlHandle, use curl_init() instead"); }
}
final class CurlMultiHandle {
    public function __construct() { throw new Error("Cannot directly construct CurlMultiHandle, use curl_multi_init() instead"); }
}
final class CurlShareHandle {
    public function __construct() { throw new Error("Cannot directly construct CurlShareHandle, use curl_share_init() instead"); }
}
class CURLFile {
    public string $name = "";
    public string $mime = "";
    public string $postname = "";
    public function __construct(string $filename, ?string $mime_type = null, ?string $posted_filename = null) {
        $this->name = $filename;
        $this->mime = $mime_type ?? "";
        $this->postname = $posted_filename ?? "";
    }
    public function getFilename(): string { return $this->name; }
    public function getMimeType(): string { return $this->mime; }
    public function getPostFilename(): string { return $this->postname; }
    public function setMimeType(string $mime_type): void { $this->mime = $mime_type; }
    public function setPostFilename(string $posted_filename): void { $this->postname = $posted_filename; }
}
class CURLStringFile {
    public string $data;
    public string $postname;
    public string $mime;
    public function __construct(string $data, string $postname, string $mime = "application/octet-stream") {
        $this->data = $data;
        $this->postname = $postname;
        $this->mime = $mime;
    }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str) -> i64 {
        consts::CONSTANTS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| *v)
            .unwrap_or_else(|| panic!("{name}"))
    }

    #[test]
    fn option_numbers_match_php_constants() {
        assert_eq!(OPT_URL, c("CURLOPT_URL"));
        assert_eq!(OPT_RETURNTRANSFER, c("CURLOPT_RETURNTRANSFER"));
        assert_eq!(OPT_FILE, c("CURLOPT_FILE"));
        assert_eq!(OPT_WRITEHEADER, c("CURLOPT_WRITEHEADER"));
        assert_eq!(OPT_HTTPHEADER, c("CURLOPT_HTTPHEADER"));
        assert_eq!(OPT_ENCODING, c("CURLOPT_ENCODING"));
        assert_eq!(OPT_XFERINFOFUNCTION, c("CURLOPT_XFERINFOFUNCTION"));
        assert_eq!(OPT_PROTOCOLS, c("CURLOPT_PROTOCOLS"));
        assert_eq!(OPT_TIMEOUT_MS, c("CURLOPT_TIMEOUT_MS"));
        assert_eq!(OPT_SHARE, c("CURLOPT_SHARE"));
        assert_eq!(OPT_PRIVATE, c("CURLOPT_PRIVATE"));
        assert_eq!(INFO_PRIVATE, c("CURLINFO_PRIVATE"));
        assert_eq!(INFO_CERTINFO, c("CURLINFO_CERTINFO"));
        assert_eq!(INFO_HEADER_OUT, c("CURLINFO_HEADER_OUT"));
    }
}
