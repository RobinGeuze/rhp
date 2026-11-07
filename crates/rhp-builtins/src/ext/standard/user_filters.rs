//! ext/standard/user_filters.c: `php_user_filter`, `StreamBucket`, the
//! `stream_bucket_*` functions, `stream_filter_register` and
//! `stream_get_filters`.

use std::cell::RefCell;
use std::rc::Rc;

use rhp_values::ops::deref;
use rhp_values::{conv, PhpArray, Thrown, Value};

use crate::main::streams::filter::{
    self, bucket_linked, bucket_new, bucket_unlink, Brigade, BucketRef, FilterOps, Status,
    PSFS_FLAG_FLUSH_CLOSE,
};
use crate::Host;

const BRIGADE_RES: &str = "userfilter.bucket brigade";
const BUCKET_RES: &str = "userfilter.bucket";

/// The classes, in PHP.
pub(crate) const PRELUDE_PHP: &str = r#"
class php_user_filter {
    public string $filtername = "";
    public mixed $params = "";
    public $stream = null;
    public function filter($in, $out, &$consumed, bool $closing): int {
        __zpp_arg("php_user_filter::filter", 1, "in", "resource", $in);
        __zpp_arg("php_user_filter::filter", 2, "out", "resource", $out);
        return PSFS_ERR_FATAL;
    }
    public function onCreate(): bool {
        if (func_num_args()) throw new ArgumentCountError('php_user_filter::onCreate() expects exactly 0 arguments, ' . func_num_args() . ' given');
        return true;
    }
    public function onClose(): void {
        if (func_num_args()) throw new ArgumentCountError('php_user_filter::onClose() expects exactly 0 arguments, ' . func_num_args() . ' given');
    }
}
final class StreamBucket {
    public $bucket;
    public string $data;
    public int $datalen;
    public int $dataLength;
}
"#;

thread_local! {
    /// BG(user_filter_map): filter pattern -> class name, in order.
    static USER_FILTERS: RefCell<Vec<(Vec<u8>, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn reset_state() {
    USER_FILTERS.with(|m| m.borrow_mut().clear());
    filter::reset_state();
}

/// A user filter: the object of the registered class.
struct UserFilter {
    obj: Value,
}

fn call_if_exists(host: &mut dyn Host, obj: &Value, method: &str) -> Option<Result<Value, Thrown>> {
    let Value::Object(h) = obj else { return None };
    let cid = h.borrow().class;
    if !host.class_has_method(cid, method.as_bytes()) {
        return None;
    }
    let mut pair = PhpArray::new();
    pair.push(obj.clone());
    pair.push(Value::str(method));
    Some(host.call_callable(&Value::Array(pair), &[]))
}

impl FilterOps for UserFilter {
    fn filter(
        &mut self,
        host: &mut dyn Host,
        stream: &Value,
        inb: &Brigade,
        outb: &Brigade,
        consumed: Option<&mut usize>,
        flags: i32,
    ) -> Status {
        let fname_warn = |host: &mut dyn Host, msg: &str| {
            filter::filter_warning(host, crate::E_WARNING, msg);
        };
        // zend_call_function does nothing while an exception is pending.
        if host.has_pending_throw() {
            return Status::ErrFatal;
        }
        let Value::Object(h) = &self.obj else {
            return Status::ErrFatal;
        };
        let class = host.class_name(h.borrow().class).unwrap_or_default();
        // Give the object a hook back to the stream.
        let has_stream = host.object_prop_exists_as(&self.obj, b"stream");
        if has_stream {
            if let Err(t) = host.reflect_set_prop(&self.obj, &class, b"stream", stream.clone()) {
                host.set_pending_throw(t);
                if !inb.is_empty() {
                    fname_warn(
                        host,
                        "Unprocessed filter buckets remaining on input brigade",
                    );
                }
                return Status::ErrFatal;
            }
        }
        let in_res = host.resource_insert(BRIGADE_RES, Box::new(inb.clone()));
        let out_res = host.resource_insert(BRIGADE_RES, Box::new(outb.clone()));
        let consumed_ref = Rc::new(RefCell::new(match &consumed {
            Some(c) => Value::Int(**c as i64),
            None => Value::Null,
        }));
        let mut pair = PhpArray::new();
        pair.push(self.obj.clone());
        pair.push(Value::str("filter"));
        let args = [
            in_res.clone(),
            out_res.clone(),
            Value::Ref(consumed_ref.clone()),
            Value::Bool(flags & PSFS_FLAG_FLUSH_CLOSE != 0),
        ];
        let mut ret = Status::ErrFatal;
        match host.call_callable(&Value::Array(pair), &args) {
            Ok(v) => ret = Status::from_long(conv::to_int(&deref(&v))),
            Err(t) => host.set_pending_throw(t),
        }
        if let Some(c) = consumed {
            *c = conv::to_int(&consumed_ref.borrow()) as usize;
        }
        if !inb.is_empty() {
            fname_warn(
                host,
                "Unprocessed filter buckets remaining on input brigade",
            );
        }
        // Don't keep the stream alive through the object.
        if has_stream {
            let pending = host.has_pending_throw();
            if let Err(t) = host.reflect_set_prop(&self.obj, &class, b"stream", Value::Null) {
                if !pending {
                    host.set_pending_throw(t);
                }
            }
        }
        host.resource_remove(&in_res);
        host.resource_remove(&out_res);
        ret
    }

    fn dtor(&mut self, host: &mut dyn Host) {
        let obj = std::mem::replace(&mut self.obj, Value::Null);
        if host.has_pending_throw() {
            return;
        }
        if let Some(Err(t)) = call_if_exists(host, &obj, "onclose") {
            host.set_pending_throw(t);
        }
    }
}

/// `user_filter_factory_create`.
pub(crate) fn create(
    host: &mut dyn Host,
    name: &[u8],
    params: Option<&Value>,
) -> Option<Box<dyn FilterOps>> {
    let class = USER_FILTERS.with(|m| {
        let m = m.borrow();
        let find = |p: &[u8]| m.iter().find(|(n, _)| n == p).map(|(_, c)| c.clone());
        find(name).or_else(|| filter::wildcards(name).iter().find_map(|w| find(w)))
    })?;
    match host.resolve_class_autoload(&class) {
        Ok(Some(_)) => {}
        Ok(None) => {
            let mut msg = format!("{}(): User-filter \"", host.active_function_name()).into_bytes();
            msg.extend_from_slice(name);
            msg.extend_from_slice(b"\" requires class \"");
            msg.extend_from_slice(&class);
            msg.extend_from_slice(b"\", but that class is not defined");
            filter::raise(host, crate::E_WARNING, &msg);
            return None;
        }
        Err(t) => {
            host.set_pending_throw(t);
            return None;
        }
    }
    let obj = match host.reflect_new_instance(&class, Vec::new(), false) {
        Ok(o) => o,
        Err(t) => {
            host.set_pending_throw(t);
            return None;
        }
    };
    let class = host.object_class_name(&obj).unwrap_or(class);
    let params = params.map(deref).unwrap_or(Value::Null);
    for (prop, val) in [(&b"filtername"[..], Value::str(name)), (b"params", params)] {
        if let Err(t) = host.reflect_set_prop(&obj, &class, prop, val) {
            host.set_pending_throw(t);
        }
    }
    match call_if_exists(host, &obj, "oncreate") {
        Some(Ok(Value::Bool(false))) => return None,
        Some(Err(t)) => host.set_pending_throw(t),
        _ => {}
    }
    Some(Box::new(UserFilter { obj }))
}

fn brigade_arg(host: &mut dyn Host, args: &[Value], fname: &str) -> Result<Brigade, Thrown> {
    let v = args.first().map(deref).unwrap_or(Value::Null);
    let bad = || {
        Thrown::type_error(format!(
            "{fname}(): supplied resource is not a valid {BRIGADE_RES} resource"
        ))
    };
    let Value::Resource(r) = &v else {
        return Err(Thrown::type_error(format!(
            "{fname}(): Argument #1 ($brigade) must be of type resource, {} given",
            crate::zend::zpp::zval_value_name(host, &v)
        )));
    };
    if r.type_name != BRIGADE_RES {
        return Err(bad());
    }
    host.resource_get(&v)
        .and_then(|b| b.downcast_ref::<Brigade>())
        .cloned()
        .ok_or_else(bad)
}

/// A `StreamBucket` object for `bucket` (a new bucket resource).
fn bucket_object(host: &mut dyn Host, bucket: BucketRef) -> Value {
    let data = bucket.borrow().buf.clone();
    let len = data.len() as i64;
    let res = host.resource_insert(BUCKET_RES, Box::new(bucket));
    host.instantiate(
        b"StreamBucket",
        vec![
            (b"bucket".to_vec(), res),
            (b"data".to_vec(), Value::str(data)),
            (b"datalen".to_vec(), Value::Int(len)),
            (b"dataLength".to_vec(), Value::Int(len)),
        ],
    )
    .unwrap_or(Value::Null)
}

/// `stream_bucket_make_writeable(resource $brigade): ?StreamBucket`.
fn fn_stream_bucket_make_writeable(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let brigade = brigade_arg(host, args, "stream_bucket_make_writeable")?;
    match brigade.pop() {
        Some(b) => Ok(bucket_object(host, b)),
        None => Ok(Value::Null),
    }
}

/// `php_stream_bucket_attach`.
fn bucket_attach(
    host: &mut dyn Host,
    args: &[Value],
    fname: &str,
    append: bool,
) -> Result<Value, Thrown> {
    let brigade = brigade_arg(host, args, fname)?;
    let obj = args.get(1).map(deref).unwrap_or(Value::Null);
    let res = host
        .reflect_get_prop(&obj, b"StreamBucket", b"bucket")?
        .map(|v| deref(&v))
        .unwrap_or(Value::Null);
    let bad = || {
        Thrown::type_error(format!(
            "{fname}(): supplied resource is not a valid {BUCKET_RES} resource"
        ))
    };
    let bucket = match &res {
        Value::Resource(r) if r.type_name == BUCKET_RES => host
            .resource_get(&res)
            .and_then(|b| b.downcast_ref::<BucketRef>())
            .cloned()
            .ok_or_else(bad)?,
        Value::Resource(_) => return Err(bad()),
        _ => {
            return Err(Thrown::type_error(format!(
                "{fname}(): supplied argument is not a valid {BUCKET_RES} resource"
            )))
        }
    };
    if let Some(data) = host.reflect_get_prop(&obj, b"StreamBucket", b"data")? {
        bucket.borrow_mut().buf = conv::to_string_bytes(&deref(&data));
    }
    if bucket_linked(&bucket) {
        bucket_unlink(&bucket);
    }
    if append {
        brigade.append(&bucket);
    } else {
        brigade.prepend(&bucket);
    }
    Ok(Value::Null)
}

fn fn_stream_bucket_append(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    bucket_attach(host, args, "stream_bucket_append", true)
}

fn fn_stream_bucket_prepend(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    bucket_attach(host, args, "stream_bucket_prepend", false)
}

/// `stream_bucket_new(resource $stream, string $buffer): StreamBucket`.
fn fn_stream_bucket_new(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    crate::ext::standard::file::stream_param(host, args, 0, "stream_bucket_new", "stream")?;
    let data = conv::to_string_bytes(&args.get(1).map(deref).unwrap_or(Value::Null));
    Ok(bucket_object(host, bucket_new(data)))
}

/// `stream_get_filters(): array`.
fn fn_stream_get_filters(_host: &mut dyn Host, _args: &[Value]) -> Result<Value, Thrown> {
    let mut a = PhpArray::new();
    for n in filter::filter_names() {
        a.push(Value::str(n));
    }
    Ok(Value::Array(a))
}

/// `stream_filter_register(string $filter_name, string $class): bool`.
fn fn_stream_filter_register(_host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let name = conv::to_string_bytes(&args.first().map(deref).unwrap_or(Value::Null));
    let class = conv::to_string_bytes(&args.get(1).map(deref).unwrap_or(Value::Null));
    for (i, (v, p)) in [(&name, "filter_name"), (&class, "class")]
        .iter()
        .enumerate()
    {
        if v.is_empty() {
            return Err(Thrown::error(
                "ValueError",
                format!(
                    "stream_filter_register(): Argument #{} (${p}) must be a non-empty string",
                    i + 1
                ),
            ));
        }
    }
    let taken = USER_FILTERS.with(|m| m.borrow().iter().any(|(n, _)| *n == name));
    if taken || !filter::register_volatile(&name) {
        return Ok(Value::Bool(false));
    }
    USER_FILTERS.with(|m| m.borrow_mut().push((name, class)));
    Ok(Value::Bool(true))
}

pub fn constants() -> Vec<(&'static str, Value)> {
    let i = Value::Int;
    vec![
        ("PSFS_PASS_ON", i(2)),
        ("PSFS_FEED_ME", i(1)),
        ("PSFS_ERR_FATAL", i(0)),
        ("PSFS_FLAG_NORMAL", i(0)),
        ("PSFS_FLAG_FLUSH_INC", i(1)),
        ("PSFS_FLAG_FLUSH_CLOSE", i(2)),
    ]
}

pub fn register(m: &mut crate::Registry) {
    crate::register(
        m,
        &[
            (
                "stream_bucket_make_writeable",
                fn_stream_bucket_make_writeable,
            ),
            ("stream_bucket_append", fn_stream_bucket_append),
            ("stream_bucket_prepend", fn_stream_bucket_prepend),
            ("stream_bucket_new", fn_stream_bucket_new),
            ("stream_get_filters", fn_stream_get_filters),
            ("stream_filter_register", fn_stream_filter_register),
        ],
    );
}
