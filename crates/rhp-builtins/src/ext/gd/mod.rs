//! The `gd` extension on libgd: a port of php-src's ext/gd/gd.c built the
//! way php's `--with-external-gd` builds it (Homebrew's php, Debian's).
//!
//! A `GdImage` object carries its `gdImagePtr` as its payload ([`Img`]),
//! destroyed with the object (php's `free_obj`); a `GdFont` from
//! `imageloadfont()` carries its font ([`Font`]). Both classes are declared
//! in [`PRELUDE_PHP`]. libgd's messages reach PHP through php's error
//! method (`shim.c`, [`rhp_gd_error`]): every libgd call runs inside
//! [`gd`], which makes the builtin's host reachable from the callback.
//! Images are written through a `gdIOCtx` whose callbacks write to PHP's
//! output or to a stream ([`Out`]), as gd.c's create_output_context.

mod sys;

use std::cell::Cell;
use std::ffi::{c_int, c_long, c_void, CStr, CString};
use std::ptr;
use std::rc::Rc;

use rhp_values::{ArrayKey, PhpArray, Value};

use crate::ext::libxml::ffi;
use crate::zend::object_handlers::{self, ClassHandlers, CloneObj, ObjectHandlers};
use crate::zend::zpp::{self, deref};
use crate::{Host, StreamKind, Thrown};
use sys::Im;

const INT_MAX: i64 = i32::MAX as i64;
const INT_MIN: i64 = i32::MIN as i64;

// php_gd.h
const IMG_GIF: i64 = 1;
const IMG_JPG: i64 = 2;
const IMG_PNG: i64 = 4;
const IMG_WBMP: i64 = 8;
const IMG_XPM: i64 = 16;
const IMG_WEBP: i64 = 32;
const IMG_BMP: i64 = 64;
const IMG_TGA: i64 = 128;
const IMG_AVIF: i64 = 256;

const FLIP_HORIZONTAL: i64 = 1;
const FLIP_VERTICAL: i64 = 2;
const FLIP_BOTH: i64 = 3;

const GD_CROP_DEFAULT: i64 = 0;
const GD_CROP_SIDES: i64 = 4;
const GD_CROP_THRESHOLD: i64 = 5;
const GD_BILINEAR_FIXED: i64 = 3;
const GD_ANTI_ALIASED: c_int = -7;

// ---------------------------------------------------------------------------
// objects
// ---------------------------------------------------------------------------

/// A `GdImage`'s image (`php_gd_image_object.image`).
pub(crate) struct Img(Cell<Im>);

impl Drop for Img {
    fn drop(&mut self) {
        let im = self.0.replace(ptr::null_mut());
        if !im.is_null() {
            unsafe { sys::gdImageDestroy(im) };
        }
    }
}

/// A `GdFont`'s font (`php_gd_font_object.font`) and the glyph data it
/// points into.
struct Font {
    font: Box<sys::GdFont>,
    _data: Vec<u8>,
}

static IMAGE_HANDLERS: ObjectHandlers = ObjectHandlers {
    clone_obj: CloneObj::None,
    compare: Some(object_handlers::not_comparable),
    ..ObjectHandlers::STD
};

static FONT_HANDLERS: ObjectHandlers = ObjectHandlers {
    clone_obj: CloneObj::None,
    ..ObjectHandlers::STD
};

/// php_gd_object_minit_helper and php_gd_font_minit_helper's handlers.
pub(crate) static CLASSES: &[ClassHandlers] = &[
    ClassHandlers {
        class: "GdImage",
        handlers: &IMAGE_HANDLERS,
        create_object: true,
    },
    ClassHandlers {
        class: "GdFont",
        handlers: &FONT_HANDLERS,
        create_object: true,
    },
];

pub const PRELUDE_PHP: &str = include_str!("prelude.php");

/// php_gd_assign_libgdimageptr_as_extgdimage: a new `GdImage` owning `im`.
fn new_image(host: &mut dyn Host, im: Im) -> Result<Value, Thrown> {
    let img = Rc::new(Img(Cell::new(im)));
    let v = host
        .instantiate(b"GdImage", Vec::new())
        .ok_or_else(|| Thrown::unsupported("gd", "cannot instantiate GdImage"))?;
    if let Some(h) = object_handlers::object_of(&v) {
        object_handlers::set_payload(&h, img);
    }
    Ok(v)
}

/// An image from libgd, or `false` for NULL.
fn image_or_false(host: &mut dyn Host, im: Im) -> Result<Value, Thrown> {
    if im.is_null() {
        Ok(Value::Bool(false))
    } else {
        new_image(host, im)
    }
}

const HAVE_PNG: i32 = 1;
const HAVE_AVIF: i32 = 2;
const HAVE_WEBP: i32 = 4;
const HAVE_JPG: i32 = 8;
const HAVE_XPM: i32 = 16;

/// Whether the linked libgd supports a format (php's HAVE_GD_* from
/// configure, probed the same way by build.rs).
fn have(bit: i32) -> bool {
    (unsafe { sys::rhp_gd_features() } & bit) != 0
}

/// php's RSHUTDOWN: libgd's FreeType cache goes with the request.
pub fn reset_state() {
    if sys::loaded() {
        unsafe { sys::gdFontCacheShutdown() };
    }
}

/// Run libgd call `f` with the host reachable from its error callback.
fn gd<R>(host: &mut dyn Host, fname: &str, f: impl FnOnce() -> R) -> Result<R, Thrown> {
    ffi::with_host(host, fname, f)
}

/// php_gd_error_method: GD_DEBUG, GD_INFO and GD_NOTICE are notices,
/// GD_WARNING a warning, anything else a fatal error, raised from the
/// running builtin.
#[no_mangle]
extern "C" fn rhp_gd_error(kind: c_int, msg: *const std::ffi::c_char, len: usize) {
    ffi::guard((), || {
        let msg = unsafe { std::slice::from_raw_parts(msg.cast::<u8>(), len) }.to_vec();
        let level = unsafe {
            if kind == sys::rhp_gd_level(1) {
                crate::E_WARNING
            } else if kind == sys::rhp_gd_level(2)
                || kind == sys::rhp_gd_level(3)
                || kind == sys::rhp_gd_level(4)
            {
                crate::E_NOTICE
            } else {
                crate::E_ERROR
            }
        };
        if ffi::aborted() {
            return;
        }
        let fname = ffi::fname();
        ffi::call_host(|h| {
            if level == crate::E_ERROR {
                let mut m = fname.clone().into_bytes();
                m.extend_from_slice(b"(): ");
                m.extend_from_slice(&msg);
                return Err(h.fatal_error(&m));
            }
            let mut m = fname.clone().into_bytes();
            m.extend_from_slice(b"(): ");
            m.extend_from_slice(&msg);
            h.emit_diagnostic(level, &m)
        });
    })
}

// ---------------------------------------------------------------------------
// arguments
// ---------------------------------------------------------------------------

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).map(deref).unwrap_or(Value::Null)
}

/// `Z_PARAM_OBJECT_OF_CLASS(.., gd_image_ce)`: the image of a `GdImage`.
fn img(host: &dyn Host, args: &[Value], i: usize, f: &str, p: &str) -> Result<Im, Thrown> {
    let v = arg(args, i);
    if object_handlers::has_handlers(host, &v, &IMAGE_HANDLERS) {
        if let Some(im) = object_handlers::payload::<Img>(&v) {
            let im = im.0.get();
            if !im.is_null() {
                return Ok(im);
            }
        }
    }
    Err(Thrown::type_error(format!(
        "{f}(): Argument #{} (${p}) must be of type GdImage, {} given",
        i + 1,
        zpp::zval_value_name(host, &v)
    )))
}

/// `Z_PARAM_LONG`. The VM's zpp has reported a null as deprecated.
fn lng(host: &mut dyn Host, args: &[Value], i: usize, f: &str, p: &str) -> Result<i64, Thrown> {
    if matches!(args.get(i).map(deref), Some(Value::Null)) && !host.caller_strict_types() {
        return Ok(0);
    }
    zpp::long_arg(host, args, i, f, p)
}

/// An optional `Z_PARAM_LONG`.
fn lng_or(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    f: &str,
    p: &str,
    default: i64,
) -> Result<i64, Thrown> {
    if args.len() <= i {
        return Ok(default);
    }
    lng(host, args, i, f, p)
}

/// `Z_PARAM_LONG_OR_NULL`.
fn lng_or_null(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    f: &str,
    p: &str,
) -> Result<Option<i64>, Thrown> {
    match args.get(i).map(deref) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => zpp::long_value(host, &v, i + 1, f, p, "?int").map(Some),
    }
}

/// `Z_PARAM_DOUBLE`.
fn dbl(host: &mut dyn Host, args: &[Value], i: usize, f: &str, p: &str) -> Result<f64, Thrown> {
    let v = arg(args, i);
    if matches!(v, Value::Null) && !host.caller_strict_types() {
        return Ok(0.0);
    }
    zpp::double_value(host, &v, i + 1, f, p)
}

/// `Z_PARAM_BOOL` (absent → `default`).
fn bln(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    f: &str,
    p: &str,
    default: bool,
) -> Result<bool, Thrown> {
    let Some(v) = args.get(i).map(deref) else {
        return Ok(default);
    };
    match v {
        Value::Bool(b) => Ok(b),
        Value::Null if !host.caller_strict_types() => Ok(false),
        _ if host.caller_strict_types() => Err(Thrown::type_error(format!(
            "{f}(): Argument #{} (${p}) must be of type bool, {} given",
            i + 1,
            zpp::zval_value_name(host, &v)
        ))),
        _ => zpp::bool_arg_or(host, args, i, f, p, default),
    }
}

fn arr(host: &dyn Host, args: &[Value], i: usize, f: &str) -> Result<PhpArray, Thrown> {
    zpp::array_param(host, args, i, f)
}

fn verr(f: &str, n: usize, p: &str, msg: &str) -> Thrown {
    zpp::value_error(f, n, p, msg)
}

/// `zval_get_long` of an array element.
fn elem_long(host: &mut dyn Host, v: &Value) -> Result<i64, Thrown> {
    zpp::zval_get_long(host, v)
}

/// `zval_get_double`.
fn elem_double(host: &mut dyn Host, v: &Value) -> Result<f64, Thrown> {
    crate::ext::standard::formatted_print::zval_get_double(host, v)
}

fn akey(s: &str) -> ArrayKey {
    ArrayKey::from_bytes(s.as_bytes())
}

fn int_list(a: &[i64]) -> Value {
    let mut r = PhpArray::new();
    for &x in a {
        r.push(Value::Int(x));
    }
    Value::Array(r)
}

fn c_bytes(p: *const std::ffi::c_char) -> Vec<u8> {
    if p.is_null() {
        Vec::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_bytes().to_vec()
    }
}

// ---------------------------------------------------------------------------
// output contexts (gd.c's "Stream Handling")
// ---------------------------------------------------------------------------

enum Sink {
    /// php_write.
    Output,
    /// A stream, closed at the end when gd.c opened it.
    Stream { res: Value, close: bool },
}

/// `php_gd_stream_ctx` (or the output ctx): a gdIOCtx and where it writes.
#[repr(C)]
struct Out {
    ctx: sys::GdIoCtx,
    sink: Sink,
    buf: Vec<u8>,
}

const STREAM_BUF: usize = 8192;

unsafe fn out_of<'a>(ctx: *mut sys::GdIoCtx) -> &'a mut Out {
    &mut *(ctx as *mut Out)
}

impl Out {
    fn new(sink: Sink) -> Box<Out> {
        Box::new(Out {
            ctx: sys::GdIoCtx {
                get_c: None,
                get_buf: None,
                put_c: Some(out_putc),
                put_buf: Some(out_putbuf),
                seek: None,
                tell: None,
                gd_free: None,
                data: ptr::null_mut(),
            },
            sink,
            buf: Vec::new(),
        })
    }

    fn ctx(&mut self) -> *mut sys::GdIoCtx {
        &mut self.ctx
    }

    /// _php_image_stream_flush.
    fn flush(&mut self, host: &mut dyn Host) {
        if let Sink::Stream { res, .. } = &self.sink {
            if !self.buf.is_empty() {
                let b = std::mem::take(&mut self.buf);
                host.stream_write(res, &b);
            }
        }
    }

    /// ctx->gd_free: flush, and close a stream gd.c opened.
    fn finish(&mut self, host: &mut dyn Host) {
        self.flush(host);
        if let Sink::Stream { res, close: true } = &self.sink {
            host.stream_close(res);
        }
    }
}

unsafe extern "C" fn out_putc(ctx: *mut sys::GdIoCtx, c: c_int) {
    ffi::guard((), || {
        let o = unsafe { out_of(ctx) };
        match &o.sink {
            Sink::Output => {
                let b = [c as u8];
                ffi::call_host(|h| {
                    h.write_out(&b);
                    Ok(())
                });
            }
            Sink::Stream { .. } => {
                if o.buf.len() == STREAM_BUF {
                    ffi::call_host(|h| {
                        o.flush(h);
                        Ok(())
                    });
                }
                o.buf.push(c as u8);
            }
        }
    })
}

unsafe extern "C" fn out_putbuf(ctx: *mut sys::GdIoCtx, buf: *const c_void, l: c_int) -> c_int {
    ffi::guard(0, || {
        let o = unsafe { out_of(ctx) };
        let data = if l <= 0 || buf.is_null() {
            &[][..]
        } else {
            unsafe { std::slice::from_raw_parts(buf.cast::<u8>(), l as usize) }
        };
        ffi::call_host(|h| {
            Ok(match &o.sink {
                Sink::Output => {
                    h.write_out(data);
                    data.len() as c_int
                }
                Sink::Stream { res, .. } => {
                    if !o.buf.is_empty() {
                        let b = std::mem::take(&mut o.buf);
                        h.stream_write(res, &b);
                    }
                    h.stream_write(res, data).unwrap_or(0) as c_int
                }
            })
        })
        .unwrap_or(0)
    })
}

/// php_stream_open_wrapper(path, mode, REPORT_ERRORS|IGNORE_PATH, NULL).
fn open_stream(
    host: &mut dyn Host,
    f: &str,
    path: &[u8],
    mode: &[u8],
    ctx: &Value,
    use_path: bool,
) -> Result<Option<Value>, Thrown> {
    if path.is_empty() {
        return Err(Thrown::error("ValueError", "Path must not be empty"));
    }
    crate::ext::standard::file::open_stream(host, f, path, mode, ctx, use_path)
}

/// create_output_context: argument `n` (1-based) names a stream resource
/// or a file to write, else (null or absent) PHP's output. `None`: php
/// returns false (an open failed, a warning was raised).
fn output_context(
    host: &mut dyn Host,
    to: &Value,
    f: &str,
    n: usize,
) -> Result<Option<Box<Out>>, Thrown> {
    let p = "file";
    match deref(to) {
        Value::Null => Ok(Some(Out::new(Sink::Output))),
        r @ Value::Resource(_) => {
            if host.stream_kind(&r).is_none() {
                return Err(Thrown::type_error(format!(
                    "{f}(): supplied resource is not a valid stream resource"
                )));
            }
            Ok(Some(Out::new(Sink::Stream {
                res: r,
                close: false,
            })))
        }
        Value::Str(s) => {
            if s.as_bytes().contains(&0) {
                return Err(Thrown::type_error(format!(
                    "{f}(): Argument #{n} (${p}) must not contain null bytes"
                )));
            }
            let res = open_stream(host, f, s.as_bytes(), b"wb", &Value::Null, false)?;
            Ok(res.map(|res| Out::new(Sink::Stream { res, close: true })))
        }
        other => Err(Thrown::type_error(format!(
            "{f}(): Argument #{n} (${p}) must be a file name or a stream resource, {} given",
            zpp::zval_value_name(host, &other)
        ))),
    }
}

/// Write with `w` through `out`, then free the context.
fn write_image(
    host: &mut dyn Host,
    f: &str,
    out: &mut Out,
    w: impl FnOnce(*mut sys::GdIoCtx),
) -> Result<(), Thrown> {
    let ctx = out.ctx();
    let r = gd(host, f, || w(ctx));
    out.finish(host);
    r
}

// ---------------------------------------------------------------------------
// gd_info, phpinfo, constants
// ---------------------------------------------------------------------------

fn version_string() -> String {
    String::from_utf8_lossy(&c_bytes(unsafe { sys::rhp_gd_version_string() })).into_owned()
}

/// PHP_MINFO_FUNCTION(gd)'s rows.
pub fn info_rows() -> Vec<(&'static str, String)> {
    let s = |v: &str| v.to_string();
    let mut rows = vec![
        ("GD Support", s("enabled")),
        ("GD headers Version", version_string()),
        (
            "GD library Version",
            String::from_utf8_lossy(&c_bytes(unsafe { sys::gdVersionString() })).into_owned(),
        ),
        ("FreeType Support", s("enabled")),
        ("FreeType Linkage", s("with freetype")),
        ("GIF Read Support", s("enabled")),
        ("GIF Create Support", s("enabled")),
    ];
    let opt = |rows: &mut Vec<_>, bit: i32, k: &'static str| {
        if have(bit) {
            rows.push((k, s("enabled")));
        }
    };
    opt(&mut rows, HAVE_JPG, "JPEG Support");
    opt(&mut rows, HAVE_PNG, "PNG Support");
    rows.push(("WBMP Support", s("enabled")));
    opt(&mut rows, HAVE_XPM, "XPM Support");
    rows.push(("XBM Support", s("enabled")));
    opt(&mut rows, HAVE_WEBP, "WebP Support");
    rows.push(("BMP Support", s("enabled")));
    opt(&mut rows, HAVE_AVIF, "AVIF Support");
    rows.push(("TGA Read Support", s("enabled")));
    rows
}

fn fn_gd_info(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    let mut a = PhpArray::new();
    a.insert(akey("GD Version"), Value::str(version_string()));
    a.insert(akey("FreeType Support"), Value::Bool(true));
    a.insert(akey("FreeType Linkage"), Value::str("with freetype"));
    for (k, v) in [
        ("GIF Read Support", true),
        ("GIF Create Support", true),
        ("JPEG Support", have(HAVE_JPG)),
        ("PNG Support", have(HAVE_PNG)),
        ("WBMP Support", true),
        ("XPM Support", have(HAVE_XPM)),
        ("XBM Support", true),
        ("WebP Support", have(HAVE_WEBP)),
        ("BMP Support", true),
        ("AVIF Support", have(HAVE_AVIF)),
        ("TGA Read Support", true),
        ("JIS-mapped Japanese Font Support", false),
    ] {
        a.insert(akey(k), Value::Bool(v));
    }
    Ok(Value::Array(a))
}

/// A constant of shim.c's table (gd.h's value).
fn c_const(name: &str) -> Option<i64> {
    let mut p = unsafe { sys::rhp_gd_consts() };
    loop {
        let c = unsafe { &*p };
        if c.name.is_null() {
            return None;
        }
        if unsafe { CStr::from_ptr(c.name) }.to_bytes() == name.as_bytes() {
            return Some(c.value);
        }
        p = unsafe { p.add(1) };
    }
}

fn gd_const(name: &str) -> i64 {
    c_const(name).unwrap_or_else(|| panic!("shim.c defines {name}"))
}

/// gd.stub.php's constants, in its order.
pub fn constants() -> Vec<(&'static str, Value)> {
    let i = Value::Int;
    let mut v: Vec<(&'static str, Value)> = vec![
        ("IMG_AVIF", i(IMG_AVIF)),
        ("IMG_GIF", i(IMG_GIF)),
        ("IMG_JPG", i(IMG_JPG)),
        ("IMG_JPEG", i(IMG_JPG)),
        ("IMG_PNG", i(IMG_PNG)),
        ("IMG_WBMP", i(IMG_WBMP)),
        ("IMG_XPM", i(IMG_XPM)),
        ("IMG_WEBP", i(IMG_WEBP)),
        ("IMG_BMP", i(IMG_BMP)),
        ("IMG_TGA", i(IMG_TGA)),
    ];
    let from_c = |v: &mut Vec<(&'static str, Value)>, names: &[&'static str]| {
        for &n in names {
            if let Some(x) = c_const(n) {
                v.push((n, Value::Int(x)));
            }
        }
    };
    from_c(
        &mut v,
        &[
            "IMG_WEBP_LOSSLESS",
            "IMG_COLOR_TILED",
            "IMG_COLOR_STYLED",
            "IMG_COLOR_BRUSHED",
            "IMG_COLOR_STYLEDBRUSHED",
            "IMG_COLOR_TRANSPARENT",
            "IMG_ARC_ROUNDED",
            "IMG_ARC_PIE",
            "IMG_ARC_CHORD",
            "IMG_ARC_NOFILL",
            "IMG_ARC_EDGED",
            "IMG_GD2_RAW",
            "IMG_GD2_COMPRESSED",
        ],
    );
    v.push(("IMG_FLIP_HORIZONTAL", i(FLIP_HORIZONTAL)));
    v.push(("IMG_FLIP_VERTICAL", i(FLIP_VERTICAL)));
    v.push(("IMG_FLIP_BOTH", i(FLIP_BOTH)));
    from_c(
        &mut v,
        &[
            "IMG_EFFECT_REPLACE",
            "IMG_EFFECT_ALPHABLEND",
            "IMG_EFFECT_NORMAL",
            "IMG_EFFECT_OVERLAY",
            "IMG_EFFECT_MULTIPLY",
            "IMG_CROP_DEFAULT",
            "IMG_CROP_TRANSPARENT",
            "IMG_CROP_BLACK",
            "IMG_CROP_WHITE",
            "IMG_CROP_SIDES",
            "IMG_CROP_THRESHOLD",
            "IMG_BELL",
            "IMG_BESSEL",
            "IMG_BILINEAR_FIXED",
            "IMG_BICUBIC",
            "IMG_BICUBIC_FIXED",
            "IMG_BLACKMAN",
            "IMG_BOX",
            "IMG_BSPLINE",
            "IMG_CATMULLROM",
            "IMG_GAUSSIAN",
            "IMG_GENERALIZED_CUBIC",
            "IMG_HERMITE",
            "IMG_HAMMING",
            "IMG_HANNING",
            "IMG_MITCHELL",
            "IMG_POWER",
            "IMG_QUADRATIC",
            "IMG_SINC",
            "IMG_NEAREST_NEIGHBOUR",
            "IMG_WEIGHTED4",
            "IMG_TRIANGLE",
            "IMG_AFFINE_TRANSLATE",
            "IMG_AFFINE_SCALE",
            "IMG_AFFINE_ROTATE",
            "IMG_AFFINE_SHEAR_HORIZONTAL",
            "IMG_AFFINE_SHEAR_VERTICAL",
        ],
    );
    v.push(("GD_BUNDLED", i(0)));
    for (n, x) in [
        ("IMG_FILTER_NEGATE", 0),
        ("IMG_FILTER_GRAYSCALE", 1),
        ("IMG_FILTER_BRIGHTNESS", 2),
        ("IMG_FILTER_CONTRAST", 3),
        ("IMG_FILTER_COLORIZE", 4),
        ("IMG_FILTER_EDGEDETECT", 5),
        ("IMG_FILTER_GAUSSIAN_BLUR", 7),
        ("IMG_FILTER_SELECTIVE_BLUR", 8),
        ("IMG_FILTER_EMBOSS", 6),
        ("IMG_FILTER_MEAN_REMOVAL", 9),
        ("IMG_FILTER_SMOOTH", 10),
        ("IMG_FILTER_PIXELATE", 11),
        ("IMG_FILTER_SCATTER", 12),
    ] {
        v.push((n, i(x)));
    }
    v.push(("GD_VERSION", Value::str(version_string())));
    from_c(
        &mut v,
        &["GD_MAJOR_VERSION", "GD_MINOR_VERSION", "GD_RELEASE_VERSION"],
    );
    v.push((
        "GD_EXTRA_VERSION",
        Value::str(c_bytes(unsafe { sys::rhp_gd_extra_version() })),
    ));
    if !have(HAVE_PNG) {
        return v;
    }
    for (n, x) in [
        ("PNG_NO_FILTER", 0x00),
        ("PNG_FILTER_NONE", 0x08),
        ("PNG_FILTER_SUB", 0x10),
        ("PNG_FILTER_UP", 0x20),
        ("PNG_FILTER_AVG", 0x40),
        ("PNG_FILTER_PAETH", 0x80),
        ("PNG_ALL_FILTERS", 0x08 | 0x10 | 0x20 | 0x40 | 0x80),
    ] {
        v.push((n, i(x)));
    }
    v
}

// ---------------------------------------------------------------------------
// fonts
// ---------------------------------------------------------------------------

/// overflow2 (gd_compat.c).
fn overflow2(host: &mut dyn Host, f: &str, a: i32, b: i32) -> Result<bool, Thrown> {
    if a <= 0 || b <= 0 {
        crate::warn(
            host,
            f,
            "One parameter to a memory allocation multiplication is negative or zero, failing operation gracefully\n",
        )?;
        return Ok(true);
    }
    if a > i32::MAX / b {
        crate::warn(
            host,
            f,
            "Product of memory allocation multiplication would exceed INT_MAX, failing operation gracefully\n",
        )?;
        return Ok(true);
    }
    Ok(false)
}

fn read_n(host: &mut dyn Host, res: &Value, want: usize) -> (Vec<u8>, bool) {
    let mut got = Vec::new();
    let mut last_ok = true;
    while got.len() < want {
        match host.stream_read(res, (want - got.len()) as i64) {
            Some(d) if !d.is_empty() => got.extend_from_slice(&d),
            _ => {
                last_ok = false;
                break;
            }
        }
    }
    (got, last_ok)
}

fn fn_imageloadfont(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imageloadfont";
    let file = zpp::path_param(host, args, 0, f, "filename")?;
    let Some(res) = open_stream(host, f, &file, b"rb", &Value::Null, false)? else {
        return Ok(Value::Bool(false));
    };
    let fail = |host: &mut dyn Host, msg: &str| -> Result<Value, Thrown> {
        host.stream_close(&res);
        crate::warn(host, f, msg)?;
        Ok(Value::Bool(false))
    };
    const HDR: usize = 16;
    let (hdr, ok) = read_n(host, &res, HDR);
    if !ok {
        let eof = host.stream_eof(&res);
        host.stream_close(&res);
        crate::warn(
            host,
            f,
            if eof {
                "End of file while reading header"
            } else {
                "Error while reading header"
            },
        )?;
        return Ok(Value::Bool(false));
    }
    let word = |i: usize| i32::from_ne_bytes([hdr[i], hdr[i + 1], hdr[i + 2], hdr[i + 3]]);
    let (mut nchars, offset, mut w, mut h) = (word(0), word(4), word(8), word(12));
    let pos = host.stream_tell(&res).unwrap_or(0);
    host.stream_seek(&res, 0, libc::SEEK_END);
    let body_size_check = (host.stream_tell(&res).unwrap_or(0) - HDR as i64) as i32;
    host.stream_seek(&res, pos, libc::SEEK_SET);
    let check = |host: &mut dyn Host, nchars: i32, w: i32, h: i32| -> Result<bool, Thrown> {
        Ok(overflow2(host, f, nchars, h)? || overflow2(host, f, nchars.wrapping_mul(h), w)?)
    };
    if check(host, nchars, w, h)? {
        return fail(host, "Error reading font, invalid font header");
    }
    let mut body_size = w.wrapping_mul(h).wrapping_mul(nchars);
    if body_size != body_size_check {
        w = w.swap_bytes();
        h = h.swap_bytes();
        nchars = nchars.swap_bytes();
        if check(host, nchars, w, h)? {
            return fail(host, "Error reading font, invalid font header");
        }
        body_size = w.wrapping_mul(h).wrapping_mul(nchars);
    }
    if body_size != body_size_check {
        return fail(host, "Error reading font");
    }
    let (mut data, ok) = read_n(host, &res, body_size as usize);
    if !ok {
        let eof = host.stream_eof(&res);
        host.stream_close(&res);
        crate::warn(
            host,
            f,
            if eof {
                "End of file while reading body"
            } else {
                "Error while reading body"
            },
        )?;
        return Ok(Value::Bool(false));
    }
    host.stream_close(&res);
    let font = Box::new(sys::GdFont {
        nchars,
        offset,
        w,
        h,
        data: data.as_mut_ptr().cast(),
    });
    let v = host
        .instantiate(b"GdFont", Vec::new())
        .ok_or_else(|| Thrown::unsupported("gd", "cannot instantiate GdFont"))?;
    if let Some(o) = object_handlers::object_of(&v) {
        object_handlers::set_payload(&o, Rc::new(Font { font, _data: data }));
    }
    Ok(v)
}

/// `Z_PARAM_OBJ_OF_CLASS_OR_LONG(.., gd_font_ce, ..)`, then
/// php_find_gd_font.
fn font_arg(
    host: &mut dyn Host,
    args: &[Value],
    i: usize,
    f: &str,
) -> Result<(*mut sys::GdFont, Option<Rc<Font>>), Thrown> {
    let v = arg(args, i);
    if let Value::Object(_) = v {
        if object_handlers::has_handlers(host, &v, &FONT_HANDLERS) {
            if let Some(font) = object_handlers::payload::<Font>(&v) {
                let p = &*font.font as *const sys::GdFont as *mut sys::GdFont;
                return Ok((p, Some(font)));
            }
        }
        return Err(Thrown::type_error(format!(
            "{f}(): Argument #{} ($font) must be of type GdFont|int, {} given",
            i + 1,
            zpp::zval_value_name(host, &v)
        )));
    }
    let n = match &v {
        Value::Int(n) => *n,
        Value::Null if !host.caller_strict_types() => 0,
        _ => zpp::long_value(host, &v, i + 1, f, "font", "GdFont|int")?,
    };
    let n = match n {
        1..=5 => n as c_int,
        n if n < 1 => 1,
        _ => 5,
    };
    Ok((unsafe { sys::rhp_gd_builtin_font(n) }, None))
}

fn font_size(host: &mut dyn Host, args: &[Value], f: &str, height: bool) -> Result<Value, Thrown> {
    let (font, _keep) = font_arg(host, args, 0, f)?;
    let font = unsafe { &*font };
    Ok(Value::Int(if height { font.h } else { font.w } as i64))
}

fn fn_imagefontwidth(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    font_size(host, args, "imagefontwidth", false)
}

fn fn_imagefontheight(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    font_size(host, args, "imagefontheight", true)
}

/// php_gdimagecharup.
fn gdimagecharup(im: Im, f: &sys::GdFont, x: i32, y: i32, c: i32, color: i32) {
    if c < f.offset || c >= f.offset + f.nchars {
        return;
    }
    let fline = (c - f.offset) * f.h * f.w;
    let mut cx = 0;
    let mut cy = 0;
    let mut py = y;
    while py > y - f.w {
        let mut px = x;
        while px < x + f.h {
            let idx = (fline + cy * f.w + cx) as isize;
            if unsafe { *f.data.offset(idx) } != 0 {
                unsafe { sys::gdImageSetPixel(im, px, py, color) };
            }
            cy += 1;
            px += 1;
        }
        cy = 0;
        cx += 1;
        py -= 1;
    }
}

/// php_imagechar: 0 imagechar, 1 imagecharup, 2 imagestring, 3
/// imagestringup.
fn imagechar(host: &mut dyn Host, args: &[Value], f: &str, mode: u8) -> Result<Value, Thrown> {
    let im = img(host, args, 0, f, "image")?;
    let (font, _keep) = font_arg(host, args, 1, f)?;
    let x = lng(host, args, 2, f, "x")?;
    let y = lng(host, args, 3, f, "y")?;
    let pname = if mode < 2 { "char" } else { "string" };
    let s = zpp::str_param(host, args, 4, f, pname)?;
    let col = lng(host, args, 5, f, "color")? as c_int;
    // estrndup + strlen: the string ends at its first NUL.
    let s: &[u8] = if mode < 2 {
        &s
    } else {
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        &s[..end]
    };
    let ch = s.first().copied().unwrap_or(0) as c_int;
    let mut x = x as u32;
    let mut y = y as u32;
    let fw = unsafe { (*font).w } as u32;
    gd(host, f, || unsafe {
        match mode {
            0 => sys::gdImageChar(im, font, x as c_int, y as c_int, ch, col),
            1 => gdimagecharup(im, &*font, x as i32, y as i32, ch, col),
            2 => {
                for &c in s {
                    sys::gdImageChar(im, font, x as c_int, y as c_int, c as c_int, col);
                    x = x.wrapping_add(fw);
                }
            }
            _ => {
                for &c in s {
                    // (int) str[i] of an unsigned char string.
                    sys::gdImageCharUp(im, font, x as c_int, y as c_int, c as c_int, col);
                    y = y.wrapping_sub(fw);
                }
            }
        }
    })?;
    Ok(Value::Bool(true))
}

fn fn_imagechar(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    imagechar(host, args, "imagechar", 0)
}
fn fn_imagecharup(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    imagechar(host, args, "imagecharup", 1)
}
fn fn_imagestring(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    imagechar(host, args, "imagestring", 2)
}
fn fn_imagestringup(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    imagechar(host, args, "imagestringup", 3)
}

// ---------------------------------------------------------------------------
// creating images
// ---------------------------------------------------------------------------

fn create(host: &mut dyn Host, args: &[Value], f: &str, truecolor: bool) -> Result<Value, Thrown> {
    let x = lng(host, args, 0, f, "width")?;
    let y = lng(host, args, 1, f, "height")?;
    if x <= 0 || x >= INT_MAX {
        return Err(verr(f, 1, "width", "must be greater than 0"));
    }
    if y <= 0 || y >= INT_MAX {
        return Err(verr(f, 2, "height", "must be greater than 0"));
    }
    let im = gd(host, f, || unsafe {
        if truecolor {
            sys::gdImageCreateTrueColor(x as c_int, y as c_int)
        } else {
            sys::gdImageCreate(x as c_int, y as c_int)
        }
    })?;
    image_or_false(host, im)
}

fn fn_imagecreatetruecolor(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    create(host, args, "imagecreatetruecolor", true)
}

fn fn_imagecreate(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    create(host, args, "imagecreate", false)
}

fn fn_imagetypes(_: &mut dyn Host, _: &[Value]) -> Result<Value, Thrown> {
    let mut r = IMG_GIF;
    for (bit, img) in [(HAVE_JPG, IMG_JPG), (HAVE_PNG, IMG_PNG)] {
        if have(bit) {
            r |= img;
        }
    }
    r |= IMG_WBMP;
    if have(HAVE_XPM) {
        r |= IMG_XPM;
    }
    if have(HAVE_WEBP) {
        r |= IMG_WEBP;
    }
    r |= IMG_BMP | IMG_TGA;
    if have(HAVE_AVIF) {
        r |= IMG_AVIF;
    }
    Ok(Value::Int(r))
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Gif,
    Png,
    Jpg,
    Wbm,
    Xbm,
    Xpm,
    Gd,
    Gd2,
    Gd2Part,
    Webp,
    Bmp,
    Tga,
    Avif,
}

impl Kind {
    fn ctx_reader(self) -> Option<sys::CtxReader> {
        Some(match self {
            Kind::Gif => sys::gdImageCreateFromGifCtx,
            Kind::Png => sys::gdImageCreateFromPngCtx,
            Kind::Jpg => sys::gdImageCreateFromJpegCtx,
            Kind::Wbm => sys::gdImageCreateFromWBMPCtx,
            Kind::Gd => sys::gdImageCreateFromGdCtx,
            Kind::Gd2 => sys::gdImageCreateFromGd2Ctx,
            Kind::Webp => sys::gdImageCreateFromWebpCtx,
            Kind::Bmp => sys::gdImageCreateFromBmpCtx,
            Kind::Tga => sys::gdImageCreateFromTgaCtx,
            Kind::Avif => sys::gdImageCreateFromAvifCtx,
            Kind::Xbm | Kind::Xpm | Kind::Gd2Part => return None,
        })
    }
}

/// A dynamic gdIOCtx over `data` (not freed by libgd), passed to `f`.
fn with_ctx(data: &[u8], f: impl FnOnce(*mut sys::GdIoCtx) -> Im) -> Im {
    let ctx =
        unsafe { sys::gdNewDynamicCtxEx(data.len() as c_int, data.as_ptr() as *mut c_void, 0) };
    if ctx.is_null() {
        return ptr::null_mut();
    }
    let im = f(ctx);
    unsafe {
        if let Some(free) = (*ctx).gd_free {
            free(ctx);
        }
    }
    im
}

/// The whole stream, from its current position (php_stream_copy_to_mem).
fn read_all(host: &mut dyn Host, res: &Value) -> Vec<u8> {
    let mut buf = Vec::new();
    loop {
        match host.stream_read(res, 8192) {
            Some(d) if !d.is_empty() => buf.extend_from_slice(&d),
            _ => break,
        }
    }
    buf
}

/// _php_image_create_from.
fn create_from(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    kind: Kind,
    tn: &str,
) -> Result<Value, Thrown> {
    let file = zpp::path_param(host, args, 0, f, "filename")?;
    let mut part = (0, 0, 0, 0);
    if kind == Kind::Gd2Part {
        let x = lng(host, args, 1, f, "x")?;
        let y = lng(host, args, 2, f, "y")?;
        let w = lng(host, args, 3, f, "width")?;
        let h = lng(host, args, 4, f, "height")?;
        if w < 1 {
            return Err(verr(f, 4, "width", "must be greater than or equal to 1"));
        }
        if h < 1 {
            return Err(verr(f, 5, "height", "must be greater than or equal to 1"));
        }
        part = (x as c_int, y as c_int, w as c_int, h as c_int);
    }
    let Some(res) = open_stream(host, f, &file, b"rb", &Value::Null, false)? else {
        return Ok(Value::Bool(false));
    };
    // A plain file is read through a FILE* in php; other streams through a
    // memory context. Only JPEG's ignore_warning and XBM tell them apart.
    let plain = host.stream_kind(&res) == Some(StreamKind::File);
    let data = read_all(host, &res);
    if data.len() as u64 > INT_MAX as u64 && !plain {
        host.stream_close(&res);
        crate::warn(
            host,
            f,
            &format!("Cannot read images with more than {INT_MAX} bytes"),
        )?;
        return Ok(Value::Bool(false));
    }
    let ignore_warning = crate::ini_value(b"gd.jpeg_ignore_warning")
        .flatten()
        .map_or(0, |v| crate::c_atol(&v)) as c_int;
    let xpm_path = CString::new(crate::zend::virtual_cwd::absolute(&file)).unwrap_or_default();
    let im = gd(host, f, || unsafe {
        match kind {
            Kind::Xbm => sys::rhp_gd_from_xbm(data.as_ptr().cast(), data.len()),
            // gdImageCreateFromXpm(file): libXpm reads the path itself.
            Kind::Xpm if plain => sys::rhp_gd_from_xpm(xpm_path.as_ptr()),
            Kind::Xpm => ptr::null_mut(),
            Kind::Gd2Part => with_ctx(&data, |c| {
                sys::gdImageCreateFromGd2PartCtx(c, part.0, part.1, part.2, part.3)
            }),
            Kind::Jpg if plain => with_ctx(&data, |c| {
                sys::gdImageCreateFromJpegCtxEx(c, ignore_warning)
            }),
            k => {
                let r = k.ctx_reader().expect("ctx reader");
                with_ctx(&data, |c| r(c))
            }
        }
    })?;
    host.stream_close(&res);
    if !im.is_null() {
        return new_image(host, im);
    }
    let mut m = b"\"".to_vec();
    m.extend_from_slice(&file);
    m.extend_from_slice(format!("\" is not a valid {tn} file").as_bytes());
    crate::warn(host, f, &m)?;
    Ok(Value::Bool(false))
}

macro_rules! create_from_fn {
    ($name:ident, $f:literal, $kind:expr, $tn:literal) => {
        fn $name(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
            create_from(host, args, $f, $kind, $tn)
        }
    };
}

create_from_fn!(
    fn_imagecreatefromgif,
    "imagecreatefromgif",
    Kind::Gif,
    "GIF"
);
create_from_fn!(
    fn_imagecreatefromjpeg,
    "imagecreatefromjpeg",
    Kind::Jpg,
    "JPEG"
);
create_from_fn!(
    fn_imagecreatefrompng,
    "imagecreatefrompng",
    Kind::Png,
    "PNG"
);
create_from_fn!(
    fn_imagecreatefromwebp,
    "imagecreatefromwebp",
    Kind::Webp,
    "WEBP"
);
create_from_fn!(
    fn_imagecreatefromxbm,
    "imagecreatefromxbm",
    Kind::Xbm,
    "XBM"
);
create_from_fn!(
    fn_imagecreatefromxpm,
    "imagecreatefromxpm",
    Kind::Xpm,
    "XPM"
);
create_from_fn!(
    fn_imagecreatefromavif,
    "imagecreatefromavif",
    Kind::Avif,
    "AVIF"
);
create_from_fn!(
    fn_imagecreatefromwbmp,
    "imagecreatefromwbmp",
    Kind::Wbm,
    "WBMP"
);
create_from_fn!(fn_imagecreatefromgd, "imagecreatefromgd", Kind::Gd, "GD");
create_from_fn!(
    fn_imagecreatefromgd2,
    "imagecreatefromgd2",
    Kind::Gd2,
    "GD2"
);
create_from_fn!(
    fn_imagecreatefromgd2part,
    "imagecreatefromgd2part",
    Kind::Gd2Part,
    "GD2"
);
create_from_fn!(
    fn_imagecreatefrombmp,
    "imagecreatefrombmp",
    Kind::Bmp,
    "BMP"
);
create_from_fn!(
    fn_imagecreatefromtga,
    "imagecreatefromtga",
    Kind::Tga,
    "TGA"
);

/// _php_image_type.
fn image_type(data: &[u8]) -> Option<Kind> {
    if data.len() < 12 {
        return None;
    }
    if data.starts_with(b"gd2") {
        return Some(Kind::Gd2);
    }
    if data.starts_with(b"\xff\xd8\xff") {
        return Some(Kind::Jpg);
    }
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(Kind::Png);
    }
    if data.starts_with(b"GIF") {
        return Some(Kind::Gif);
    }
    if data.starts_with(b"BM") {
        return Some(Kind::Bmp);
    }
    if data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
        return Some(Kind::Webp);
    }
    if crate::ext::standard::image::is_image_avif(data) {
        return Some(Kind::Avif);
    }
    if data[0] == 0 {
        return Some(Kind::Wbm);
    }
    None
}

fn fn_imagecreatefromstring(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagecreatefromstring";
    let data = zpp::zstr_param(host, args, 0, f, "data")?;
    let data = data.as_bytes();
    let Some(kind) = image_type(data) else {
        crate::warn(host, f, "Data is not in a recognized format")?;
        return Ok(Value::Bool(false));
    };
    let tn = match kind {
        Kind::Jpg => "JPEG",
        Kind::Png => "PNG",
        Kind::Gif => "GIF",
        Kind::Wbm => "WBMP",
        Kind::Gd2 => "GD2",
        Kind::Bmp => "BMP",
        Kind::Webp => "WEBP",
        _ => "AVIF",
    };
    let missing = match kind {
        Kind::Jpg => !have(HAVE_JPG),
        Kind::Png => !have(HAVE_PNG),
        Kind::Webp => !have(HAVE_WEBP),
        Kind::Avif => !have(HAVE_AVIF),
        _ => false,
    };
    if missing {
        crate::warn(host, f, &format!("No {tn} support in this PHP build"))?;
        return Ok(Value::Bool(false));
    }
    let reader = kind.ctx_reader().expect("ctx reader");
    // _php_image_create_from_string
    let im = gd(host, f, || with_ctx(data, |c| unsafe { reader(c) }))?;
    if im.is_null() {
        crate::warn(host, f, &format!("Passed data is not in \"{tn}\" format"))?;
        crate::warn(host, f, "Couldn't create GD Image Stream out of Data")?;
        return Ok(Value::Bool(false));
    }
    new_image(host, im)
}

// ---------------------------------------------------------------------------
// writing images
// ---------------------------------------------------------------------------

/// The `$file` argument: absent or null is PHP's output.
fn file_arg(args: &[Value], i: usize) -> Value {
    arg(args, i)
}

fn fn_imagegif(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagegif";
    let im = img(host, args, 0, f, "image")?;
    let Some(mut out) = output_context(host, &file_arg(args, 1), f, 2)? else {
        return Ok(Value::Bool(false));
    };
    write_image(host, f, &mut out, |c| unsafe { sys::gdImageGifCtx(im, c) })?;
    Ok(Value::Bool(true))
}

fn fn_imagepng(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagepng";
    let im = img(host, args, 0, f, "image")?;
    let to = file_arg(args, 1);
    let quality = lng_or(host, args, 2, f, "quality", -1)?;
    let _filters = lng_or(host, args, 3, f, "filters", -1)?;
    let Some(mut out) = output_context(host, &to, f, 2)? else {
        return Ok(Value::Bool(false));
    };
    if !(-1..=9).contains(&quality) {
        out.finish(host);
        return Err(verr(f, 3, "quality", "must be between -1 and 9"));
    }
    write_image(host, f, &mut out, |c| unsafe {
        sys::gdImagePngCtxEx(im, c, quality as c_int)
    })?;
    Ok(Value::Bool(true))
}

fn fn_imagewebp(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagewebp";
    let im = img(host, args, 0, f, "image")?;
    let to = file_arg(args, 1);
    let quality = lng_or(host, args, 2, f, "quality", -1)?;
    let Some(mut out) = output_context(host, &to, f, 2)? else {
        return Ok(Value::Bool(false));
    };
    if quality < -1 {
        out.finish(host);
        return Err(verr(f, 3, "quality", "must be greater than or equal to -1"));
    }
    write_image(host, f, &mut out, |c| unsafe {
        sys::gdImageWebpCtx(im, c, quality as c_int)
    })?;
    Ok(Value::Bool(true))
}

fn fn_imageavif(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imageavif";
    let im = img(host, args, 0, f, "image")?;
    let to = file_arg(args, 1);
    let quality = lng_or(host, args, 2, f, "quality", -1)?;
    let mut speed = lng_or(host, args, 3, f, "speed", -1)?;
    let Some(mut out) = output_context(host, &to, f, 2)? else {
        return Ok(Value::Bool(false));
    };
    if !(-1..=100).contains(&quality) {
        out.finish(host);
        return Err(verr(f, 3, "quality", "must be between -1 and 100"));
    }
    if !(-1..=10).contains(&speed) {
        out.finish(host);
        return Err(verr(f, 4, "speed", "must be between -1 and 10"));
    } else if speed == -1 {
        speed = 6;
    }
    write_image(host, f, &mut out, |c| unsafe {
        sys::gdImageAvifCtx(im, c, quality as c_int, speed as c_int)
    })?;
    Ok(Value::Bool(true))
}

fn fn_imagejpeg(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagejpeg";
    let im = img(host, args, 0, f, "image")?;
    let to = file_arg(args, 1);
    let quality = lng_or(host, args, 2, f, "quality", -1)?;
    let Some(mut out) = output_context(host, &to, f, 2)? else {
        return Ok(Value::Bool(false));
    };
    if !(-1..=100).contains(&quality) {
        out.finish(host);
        return Err(verr(f, 3, "quality", "must be at between -1 and 100"));
    }
    write_image(host, f, &mut out, |c| unsafe {
        sys::gdImageJpegCtx(im, c, quality as c_int)
    })?;
    Ok(Value::Bool(true))
}

/// The first black palette entry (imagewbmp's and imagexbm's default).
fn first_black(im: Im) -> i64 {
    unsafe {
        let total = sys::rhp_gd_colors_total(im);
        let mut i = 0;
        while i < total {
            if sys::rhp_gd_red(im, i) == 0
                && sys::rhp_gd_green(im, i) == 0
                && sys::rhp_gd_blue(im, i) == 0
            {
                break;
            }
            i += 1;
        }
        i as i64
    }
}

fn fn_imagewbmp(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagewbmp";
    let im = img(host, args, 0, f, "image")?;
    let to = file_arg(args, 1);
    let fg = lng_or_null(host, args, 2, f, "foreground_color")?;
    let Some(mut out) = output_context(host, &to, f, 2)? else {
        return Ok(Value::Bool(false));
    };
    let fg = fg.unwrap_or_else(|| first_black(im));
    write_image(host, f, &mut out, |c| unsafe {
        sys::gdImageWBMPCtx(im, fg as c_int, c)
    })?;
    Ok(Value::Bool(true))
}

fn fn_imagebmp(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagebmp";
    let im = img(host, args, 0, f, "image")?;
    let to = file_arg(args, 1);
    let compressed = bln(host, args, 2, f, "compressed", true)?;
    let Some(mut out) = output_context(host, &to, f, 2)? else {
        return Ok(Value::Bool(false));
    };
    write_image(host, f, &mut out, |c| unsafe {
        sys::gdImageBmpCtx(im, c, compressed as c_int)
    })?;
    Ok(Value::Bool(true))
}

fn fn_imagexbm(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagexbm";
    let im = img(host, args, 0, f, "image")?;
    let file = match arg(args, 1) {
        Value::Null => None,
        _ => Some(zpp::path_param(host, args, 1, f, "filename")?),
    };
    let fg = lng_or_null(host, args, 2, f, "foreground_color")?;
    let mut out = match &file {
        Some(path) => {
            let Some(res) = open_stream(host, f, path, b"wb", &Value::Null, false)? else {
                return Ok(Value::Bool(false));
            };
            Out::new(Sink::Stream { res, close: true })
        }
        None => Out::new(Sink::Output),
    };
    let fg = fg.unwrap_or_else(|| first_black(im));
    let name = CString::new(file.unwrap_or_default()).unwrap_or_default();
    write_image(host, f, &mut out, |c| unsafe {
        sys::gdImageXbmCtx(im, name.as_ptr() as *mut _, fg as c_int, c)
    })?;
    Ok(Value::Bool(true))
}

/// _php_image_output: imagegd and imagegd2, written to a file or to PHP's
/// output.
fn image_output(host: &mut dyn Host, args: &[Value], f: &str, gd2: bool) -> Result<Value, Thrown> {
    let im = img(host, args, 0, f, "image")?;
    let file = match arg(args, 1) {
        Value::Null => None,
        _ => Some(zpp::path_param(host, args, 1, f, "file")?),
    };
    let (mut quality, kind) = if gd2 {
        (
            lng_or(host, args, 2, f, "chunk_size", 128)?,
            lng_or(host, args, 3, f, "mode", 1)?,
        )
    } else {
        (128, 1)
    };
    if !(INT_MIN..=INT_MAX).contains(&quality) {
        crate::warn(
            host,
            f,
            &format!("Argument #3 ($chunk_size) must be between {INT_MIN} and {INT_MAX}"),
        )?;
        return Ok(Value::Bool(false));
    }
    if quality == -1 {
        quality = 128;
    }
    // PHP_GD_CHECK_OPEN_BASEDIR and VCWD_FOPEN(file, "wb") come before
    // libgd writes anything.
    let file = match file.filter(|p| !p.is_empty()) {
        Some(path) => {
            if !crate::main::fopen_wrappers::check(host, f, &path, true)? {
                crate::warn(host, f, "Invalid filename")?;
                return Ok(Value::Bool(false));
            }
            let resolved = crate::zend::virtual_cwd::resolve(&path).into_owned();
            let fp = crate::zend::virtual_cwd::library_created(&resolved, 0o666, || {
                std::fs::File::create(crate::zend::virtual_cwd::os_path(&resolved)).ok()
            });
            let Some(fp) = fp else {
                let mut m = b"Unable to open \"".to_vec();
                m.extend_from_slice(&path);
                m.extend_from_slice(b"\" for writing");
                crate::warn(host, f, &m)?;
                return Ok(Value::Bool(false));
            };
            Some(fp)
        }
        None => None,
    };
    let bytes = gd(host, f, || unsafe {
        let mut size: c_int = 0;
        let p = if gd2 {
            sys::gdImageGd2Ptr(im, quality as c_int, kind as c_int, &mut size)
        } else {
            sys::gdImageGdPtr(im, &mut size)
        };
        if p.is_null() {
            return Vec::new();
        }
        let v = std::slice::from_raw_parts(p.cast::<u8>(), size.max(0) as usize).to_vec();
        sys::gdFree(p);
        v
    })?;
    match file {
        Some(mut fp) => {
            use std::io::Write;
            let _ = fp.write_all(&bytes);
        }
        None => host.write_out(&bytes),
    }
    Ok(Value::Bool(true))
}

fn fn_imagegd(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    image_output(host, args, "imagegd", false)
}

fn fn_imagegd2(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    image_output(host, args, "imagegd2", true)
}

// ---------------------------------------------------------------------------
// simple image functions
// ---------------------------------------------------------------------------

fn fn_imagedestroy(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    img(host, args, 0, "imagedestroy", "image")?;
    Ok(Value::Bool(true))
}

fn fn_imageistruecolor(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let im = img(host, args, 0, "imageistruecolor", "image")?;
    Ok(Value::Bool(unsafe { sys::rhp_gd_truecolor(im) } != 0))
}

fn fn_imagesx(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let im = img(host, args, 0, "imagesx", "image")?;
    Ok(Value::Int(unsafe { sys::rhp_gd_sx(im) } as i64))
}

fn fn_imagesy(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let im = img(host, args, 0, "imagesy", "image")?;
    Ok(Value::Int(unsafe { sys::rhp_gd_sy(im) } as i64))
}

fn fn_imagecolorstotal(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let im = img(host, args, 0, "imagecolorstotal", "image")?;
    Ok(Value::Int(unsafe { sys::rhp_gd_colors_total(im) } as i64))
}

fn fn_imagesetstyle(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagesetstyle";
    let im = img(host, args, 0, f, "image")?;
    let styles = arr(host, args, 1, f)?;
    if styles.is_empty() {
        return Err(verr(f, 2, "style", "must not be empty"));
    }
    let mut v = Vec::with_capacity(styles.len());
    for (_, item) in styles.iter() {
        v.push(elem_long(host, item)? as c_int);
    }
    gd(host, f, || unsafe {
        sys::gdImageSetStyle(im, v.as_mut_ptr(), v.len() as c_int)
    })?;
    Ok(Value::Bool(true))
}

fn fn_imagetruecolortopalette(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagetruecolortopalette";
    let im = img(host, args, 0, f, "image")?;
    let dither = bln(host, args, 1, f, "dither", false)?;
    let n = lng(host, args, 2, f, "num_colors")?;
    if n <= 0 || n > INT_MAX {
        return Err(verr(
            f,
            3,
            "num_colors",
            &format!("must be greater than 0 and less than {INT_MAX}"),
        ));
    }
    let ok = gd(host, f, || unsafe {
        sys::gdImageTrueColorToPalette(im, dither as c_int, n as c_int)
    })?;
    if ok != 0 {
        Ok(Value::Bool(true))
    } else {
        crate::warn(host, f, "Couldn't convert to palette")?;
        Ok(Value::Bool(false))
    }
}

fn fn_imagepalettetotruecolor(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagepalettetotruecolor";
    let im = img(host, args, 0, f, "image")?;
    let r = gd(host, f, || unsafe { sys::gdImagePaletteToTrueColor(im) })?;
    Ok(Value::Bool(r != 0))
}

fn fn_imagecolormatch(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagecolormatch";
    let im1 = img(host, args, 0, f, "image1")?;
    let im2 = img(host, args, 1, f, "image2")?;
    let r = gd(host, f, || unsafe { sys::gdImageColorMatch(im1, im2) })?;
    match r {
        -1 => Err(verr(f, 1, "image1", "must be TrueColor")),
        -2 => Err(verr(f, 2, "image2", "must be Palette")),
        -3 => Err(verr(
            f,
            2,
            "image2",
            "must be the same size as argument #1 ($im1)",
        )),
        -4 => Err(verr(f, 2, "image2", "must have at least one color")),
        _ => Ok(Value::Bool(true)),
    }
}

/// An image function taking only ints after the image: `call(im, ints)`.
fn ints(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    names: &[&str],
) -> Result<(Im, Vec<i64>), Thrown> {
    let im = img(host, args, 0, f, "image")?;
    let mut v = Vec::with_capacity(names.len());
    for (k, p) in names.iter().enumerate() {
        v.push(lng(host, args, k + 1, f, p)?);
    }
    Ok((im, v))
}

fn fn_imagesetthickness(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagesetthickness";
    let (im, a) = ints(host, args, f, &["thickness"])?;
    gd(host, f, || unsafe {
        sys::gdImageSetThickness(im, a[0] as c_int)
    })?;
    Ok(Value::Bool(true))
}

fn fn_imagefilledellipse(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagefilledellipse";
    let (im, a) = ints(
        host,
        args,
        f,
        &["center_x", "center_y", "width", "height", "color"],
    )?;
    if a[2] < 0 || a[2] > INT_MAX {
        return Err(verr(
            f,
            4,
            "width",
            &format!("must be between 0 and {INT_MAX}"),
        ));
    }
    let c = |i: usize| a[i] as c_int;
    gd(host, f, || unsafe {
        sys::gdImageFilledEllipse(im, c(0), c(1), c(2), c(3), c(4))
    })?;
    Ok(Value::Bool(true))
}

/// php's angle normalisation: a negative angle is reduced mod 360.
fn angle(v: i64) -> c_int {
    let e = v as c_int;
    if e < 0 {
        e % 360
    } else {
        e
    }
}

fn fn_imagefilledarc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagefilledarc";
    let (im, a) = ints(
        host,
        args,
        f,
        &[
            "center_x",
            "center_y",
            "width",
            "height",
            "start_angle",
            "end_angle",
            "color",
            "style",
        ],
    )?;
    let c = |i: usize| a[i] as c_int;
    let (st, e) = (angle(a[4]), angle(a[5]));
    gd(host, f, || unsafe {
        sys::gdImageFilledArc(im, c(0), c(1), c(2), c(3), st, e, c(6), c(7))
    })?;
    Ok(Value::Bool(true))
}

fn fn_imagearc(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagearc";
    let (im, a) = ints(
        host,
        args,
        f,
        &[
            "center_x",
            "center_y",
            "width",
            "height",
            "start_angle",
            "end_angle",
            "color",
        ],
    )?;
    let c = |i: usize| a[i] as c_int;
    let (st, e) = (angle(a[4]), angle(a[5]));
    gd(host, f, || unsafe {
        sys::gdImageArc(im, c(0), c(1), c(2), c(3), st, e, c(6))
    })?;
    Ok(Value::Bool(true))
}

fn fn_imageellipse(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imageellipse";
    let (im, a) = ints(
        host,
        args,
        f,
        &["center_x", "center_y", "width", "height", "color"],
    )?;
    let c = |i: usize| a[i] as c_int;
    gd(host, f, || unsafe {
        sys::gdImageEllipse(im, c(0), c(1), c(2), c(3), c(4))
    })?;
    Ok(Value::Bool(true))
}

fn fn_imagealphablending(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagealphablending";
    let im = img(host, args, 0, f, "image")?;
    let b = bln(host, args, 1, f, "enable", false)?;
    gd(host, f, || unsafe {
        sys::gdImageAlphaBlending(im, b as c_int)
    })?;
    Ok(Value::Bool(true))
}

fn fn_imagesavealpha(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagesavealpha";
    let im = img(host, args, 0, f, "image")?;
    let b = bln(host, args, 1, f, "enable", false)?;
    gd(host, f, || unsafe { sys::gdImageSaveAlpha(im, b as c_int) })?;
    Ok(Value::Bool(true))
}

fn fn_imagelayereffect(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagelayereffect";
    let (im, a) = ints(host, args, f, &["effect"])?;
    gd(host, f, || unsafe {
        sys::gdImageAlphaBlending(im, a[0] as c_int)
    })?;
    Ok(Value::Bool(true))
}

/// CHECK_RGBA_RANGE for the components starting at argument `first`
/// (1-based); `alpha`: the last one is alpha (0..127).
fn check_rgba(f: &str, a: &[i64], first: usize, names: &[&str]) -> Result<(), Thrown> {
    for (k, (&v, &p)) in a.iter().zip(names).enumerate() {
        let max = if p == "alpha" { 127 } else { 255 };
        if v < 0 || v > max {
            return Err(verr(
                f,
                first + k,
                p,
                &format!("must be between 0 and {max} (inclusive)"),
            ));
        }
    }
    Ok(())
}

type ColorFn = unsafe extern "C" fn(Im, c_int, c_int, c_int) -> c_int;
type ColorAlphaFn = unsafe extern "C" fn(Im, c_int, c_int, c_int, c_int) -> c_int;

/// The color lookups: `rgb` (r, g, b) or with `alpha`. `false_below_0`:
/// imagecolorallocate's false for a failed allocation.
fn color3(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    g: ColorFn,
    false_below_0: bool,
) -> Result<Value, Thrown> {
    let names = ["red", "green", "blue"];
    let (im, a) = ints(host, args, f, &names)?;
    check_rgba(f, &a, 2, &names)?;
    let r = gd(host, f, || unsafe {
        g(im, a[0] as c_int, a[1] as c_int, a[2] as c_int)
    })?;
    if false_below_0 && r < 0 {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Int(r as i64))
}

fn color4(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    g: ColorAlphaFn,
    false_below_0: bool,
) -> Result<Value, Thrown> {
    let names = ["red", "green", "blue", "alpha"];
    let (im, a) = ints(host, args, f, &names)?;
    check_rgba(f, &a, 2, &names)?;
    let r = gd(host, f, || unsafe {
        g(
            im,
            a[0] as c_int,
            a[1] as c_int,
            a[2] as c_int,
            a[3] as c_int,
        )
    })?;
    if false_below_0 && r < 0 {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Int(r as i64))
}

fn fn_imagecolorallocatealpha(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    color4(
        host,
        args,
        "imagecolorallocatealpha",
        sys::gdImageColorAllocateAlpha,
        true,
    )
}
fn fn_imagecolorresolvealpha(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    color4(
        host,
        args,
        "imagecolorresolvealpha",
        sys::gdImageColorResolveAlpha,
        false,
    )
}
fn fn_imagecolorclosestalpha(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    color4(
        host,
        args,
        "imagecolorclosestalpha",
        sys::gdImageColorClosestAlpha,
        false,
    )
}
fn fn_imagecolorexactalpha(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    color4(
        host,
        args,
        "imagecolorexactalpha",
        sys::gdImageColorExactAlpha,
        false,
    )
}
fn fn_imagecolorallocate(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    color3(
        host,
        args,
        "imagecolorallocate",
        sys::gdImageColorAllocate,
        true,
    )
}
fn fn_imagecolorclosest(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    color3(
        host,
        args,
        "imagecolorclosest",
        sys::gdImageColorClosest,
        false,
    )
}
fn fn_imagecolorclosesthwb(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    color3(
        host,
        args,
        "imagecolorclosesthwb",
        sys::gdImageColorClosestHWB,
        false,
    )
}
fn fn_imagecolorresolve(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    color3(
        host,
        args,
        "imagecolorresolve",
        sys::gdImageColorResolve,
        false,
    )
}
fn fn_imagecolorexact(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    color3(host, args, "imagecolorexact", sys::gdImageColorExact, false)
}

/// The copies: dst, src, then ints.
fn two_images(
    host: &mut dyn Host,
    args: &[Value],
    f: &str,
    names: &[&str],
) -> Result<(Im, Im, Vec<c_int>), Thrown> {
    let dst = img(host, args, 0, f, "dst_image")?;
    let src = img(host, args, 1, f, "src_image")?;
    let mut v = Vec::with_capacity(names.len());
    for (k, p) in names.iter().enumerate() {
        v.push(lng(host, args, k + 2, f, p)? as c_int);
    }
    Ok((dst, src, v))
}

const COPY_NAMES: [&str; 6] = [
    "dst_x",
    "dst_y",
    "src_x",
    "src_y",
    "src_width",
    "src_height",
];
const RESIZE_NAMES: [&str; 8] = [
    "dst_x",
    "dst_y",
    "src_x",
    "src_y",
    "dst_width",
    "dst_height",
    "src_width",
    "src_height",
];

fn fn_imagecopyresampled(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagecopyresampled";
    let (dst, src, a) = two_images(host, args, f, &RESIZE_NAMES)?;
    gd(host, f, || unsafe {
        sys::gdImageCopyResampled(dst, src, a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7])
    })?;
    Ok(Value::Bool(true))
}

fn fn_imagecopyresized(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagecopyresized";
    let (dst, src, a) = two_images(host, args, f, &RESIZE_NAMES)?;
    for (k, p) in [
        (4, "dst_width"),
        (5, "dst_height"),
        (6, "src_width"),
        (7, "src_height"),
    ] {
        if a[k] <= 0 {
            return Err(verr(f, k + 3, p, "must be greater than 0"));
        }
    }
    gd(host, f, || unsafe {
        sys::gdImageCopyResized(dst, src, a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7])
    })?;
    Ok(Value::Bool(true))
}

fn fn_imagecopy(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagecopy";
    let (dst, src, a) = two_images(host, args, f, &COPY_NAMES)?;
    gd(host, f, || unsafe {
        sys::gdImageCopy(dst, src, a[0], a[1], a[2], a[3], a[4], a[5])
    })?;
    Ok(Value::Bool(true))
}

fn copy_merge(host: &mut dyn Host, args: &[Value], f: &str, gray: bool) -> Result<Value, Thrown> {
    let mut names = COPY_NAMES.to_vec();
    names.push("pct");
    let (dst, src, a) = two_images(host, args, f, &names)?;
    gd(host, f, || unsafe {
        if gray {
            sys::gdImageCopyMergeGray(dst, src, a[0], a[1], a[2], a[3], a[4], a[5], a[6])
        } else {
            sys::gdImageCopyMerge(dst, src, a[0], a[1], a[2], a[3], a[4], a[5], a[6])
        }
    })?;
    Ok(Value::Bool(true))
}

fn fn_imagecopymerge(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    copy_merge(host, args, "imagecopymerge", false)
}

fn fn_imagecopymergegray(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    copy_merge(host, args, "imagecopymergegray", true)
}

fn fn_imagerotate(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagerotate";
    let im = img(host, args, 0, f, "image")?;
    let degrees = dbl(host, args, 1, f, "angle")?;
    let color = lng(host, args, 2, f, "background_color")?;
    let (lo, hi) = (i32::MIN / 100, i32::MAX / 100);
    if degrees < lo as f64 || degrees > hi as f64 {
        return Err(verr(
            f,
            2,
            "angle",
            &format!("must be between {lo} and {hi}"),
        ));
    }
    let r = gd(host, f, || unsafe {
        sys::gdImageRotateInterpolated(im, degrees as f32, color as c_int)
    })?;
    image_or_false(host, r)
}

fn image_pair(host: &mut dyn Host, args: &[Value], f: &str, p2: &str) -> Result<(Im, Im), Thrown> {
    let a = img(host, args, 0, f, "image")?;
    let b = img(host, args, 1, f, p2)?;
    Ok((a, b))
}

fn fn_imagesettile(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagesettile";
    let (im, t) = image_pair(host, args, f, "tile")?;
    gd(host, f, || unsafe { sys::gdImageSetTile(im, t) })?;
    Ok(Value::Bool(true))
}

fn fn_imagesetbrush(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagesetbrush";
    let (im, t) = image_pair(host, args, f, "brush")?;
    gd(host, f, || unsafe { sys::gdImageSetBrush(im, t) })?;
    Ok(Value::Bool(true))
}

fn fn_imagepalettecopy(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagepalettecopy";
    let dst = img(host, args, 0, f, "dst")?;
    let src = img(host, args, 1, f, "src")?;
    gd(host, f, || unsafe { sys::gdImagePaletteCopy(dst, src) })?;
    Ok(Value::Null)
}

fn fn_imagecolorat(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagecolorat";
    let (im, a) = ints(host, args, f, &["x", "y"])?;
    let mut out: c_long = 0;
    if unsafe { sys::rhp_gd_color_at(im, a[0] as c_long, a[1] as c_long, &mut out) } != 0 {
        return Ok(Value::Int(out as i64));
    }
    crate::notice(host, f, &format!("{},{} is out of bounds", a[0], a[1]))?;
    Ok(Value::Bool(false))
}

fn fn_imagecolordeallocate(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagecolordeallocate";
    let (im, a) = ints(host, args, f, &["color"])?;
    if unsafe { sys::rhp_gd_truecolor(im) } != 0 {
        return Ok(Value::Bool(true));
    }
    let col = a[0] as c_int;
    let total = unsafe { sys::rhp_gd_colors_total(im) };
    if col >= 0 && col < total {
        gd(host, f, || unsafe { sys::gdImageColorDeallocate(im, col) })?;
        Ok(Value::Bool(true))
    } else {
        Err(verr(
            f,
            2,
            "color",
            &format!("must be between 0 and {total}"),
        ))
    }
}

fn fn_imagecolorset(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagecolorset";
    let im = img(host, args, 0, f, "image")?;
    let color = lng(host, args, 1, f, "color")?;
    let names = ["red", "green", "blue"];
    let mut a = Vec::new();
    for (k, p) in names.iter().enumerate() {
        a.push(lng(host, args, k + 2, f, p)?);
    }
    a.push(lng_or(host, args, 5, f, "alpha", 0)?);
    check_rgba(f, &a, 3, &["red", "green", "blue", "alpha"])?;
    let col = color as c_int;
    if col >= 0 && col < unsafe { sys::rhp_gd_colors_total(im) } {
        unsafe {
            sys::rhp_gd_set_palette(
                im,
                col,
                a[0] as c_int,
                a[1] as c_int,
                a[2] as c_int,
                a[3] as c_int,
            )
        };
        Ok(Value::Null)
    } else {
        Ok(Value::Bool(false))
    }
}

fn fn_imagecolorsforindex(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagecolorsforindex";
    let (im, a) = ints(host, args, f, &["color"])?;
    let col = a[0] as c_int;
    let tc = unsafe { sys::rhp_gd_truecolor(im) } != 0;
    if (col >= 0 && tc) || (!tc && col >= 0 && col < unsafe { sys::rhp_gd_colors_total(im) }) {
        let mut r = PhpArray::new();
        unsafe {
            r.insert(akey("red"), Value::Int(sys::rhp_gd_red(im, col) as i64));
            r.insert(akey("green"), Value::Int(sys::rhp_gd_green(im, col) as i64));
            r.insert(akey("blue"), Value::Int(sys::rhp_gd_blue(im, col) as i64));
            r.insert(akey("alpha"), Value::Int(sys::rhp_gd_alpha(im, col) as i64));
        }
        Ok(Value::Array(r))
    } else {
        Err(verr(f, 2, "color", "is out of range"))
    }
}

fn fn_imagegammacorrect(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagegammacorrect";
    let im = img(host, args, 0, f, "image")?;
    let input = dbl(host, args, 1, f, "input_gamma")?;
    let output = dbl(host, args, 2, f, "output_gamma")?;
    if input <= 0.0 {
        return Err(verr(f, 2, "input_gamma", "must be greater than 0"));
    }
    if !input.is_finite() {
        return Err(verr(f, 2, "input_gamma", "must be finite"));
    }
    if output <= 0.0 {
        return Err(verr(f, 3, "output_gamma", "must be greater than 0"));
    }
    if !output.is_finite() {
        return Err(verr(f, 3, "output_gamma", "must be finite"));
    }
    let gamma = input / output;
    let g = |c: c_int| ((((c as f64) / 255.0).powf(gamma) * 255.0) + 0.5) as c_int;
    gd(host, f, || unsafe {
        if sys::rhp_gd_truecolor(im) != 0 {
            for y in 0..sys::rhp_gd_sy(im) {
                for x in 0..sys::rhp_gd_sx(im) {
                    let c = sys::gdImageGetPixel(im, x, y);
                    let (r, gr, b, a) = (
                        (c >> 16) & 0xff,
                        (c >> 8) & 0xff,
                        c & 0xff,
                        (c & 0x7f00_0000) >> 24,
                    );
                    let v = (a << 24) + (g(r) << 16) + (g(gr) << 8) + g(b);
                    sys::gdImageSetPixel(im, x, y, v);
                }
            }
        } else {
            for i in 0..sys::rhp_gd_colors_total(im) {
                sys::rhp_gd_set_rgb(
                    im,
                    i,
                    g(sys::rhp_gd_red(im, i)),
                    g(sys::rhp_gd_green(im, i)),
                    g(sys::rhp_gd_blue(im, i)),
                );
            }
        }
    })?;
    Ok(Value::Bool(true))
}

fn fn_imagesetpixel(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagesetpixel";
    let (im, a) = ints(host, args, f, &["x", "y", "color"])?;
    gd(host, f, || unsafe {
        sys::gdImageSetPixel(im, a[0] as c_int, a[1] as c_int, a[2] as c_int)
    })?;
    Ok(Value::Bool(true))
}

const LINE: [&str; 5] = ["x1", "y1", "x2", "y2", "color"];

fn fn_imageline(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imageline";
    let (im, a) = ints(host, args, f, &LINE)?;
    let c = |i: usize| a[i] as c_int;
    gd(host, f, || unsafe {
        let mut col = c(4);
        if sys::rhp_gd_aa(im) != 0 {
            sys::gdImageSetAntiAliased(im, col);
            col = GD_ANTI_ALIASED;
        }
        sys::gdImageLine(im, c(0), c(1), c(2), c(3), col)
    })?;
    Ok(Value::Bool(true))
}

type RectFn = unsafe extern "C" fn(Im, c_int, c_int, c_int, c_int, c_int);

fn line_like(host: &mut dyn Host, args: &[Value], f: &str, g: RectFn) -> Result<Value, Thrown> {
    let (im, a) = ints(host, args, f, &LINE)?;
    let c = |i: usize| a[i] as c_int;
    gd(host, f, || unsafe { g(im, c(0), c(1), c(2), c(3), c(4)) })?;
    Ok(Value::Bool(true))
}

fn fn_imagedashedline(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    line_like(host, args, "imagedashedline", sys::gdImageDashedLine)
}
fn fn_imagerectangle(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    line_like(host, args, "imagerectangle", sys::gdImageRectangle)
}
fn fn_imagefilledrectangle(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    line_like(
        host,
        args,
        "imagefilledrectangle",
        sys::gdImageFilledRectangle,
    )
}

fn fn_imagefilltoborder(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagefilltoborder";
    let (im, a) = ints(host, args, f, &["x", "y", "border_color", "color"])?;
    let c = |i: usize| a[i] as c_int;
    gd(host, f, || unsafe {
        sys::gdImageFillToBorder(im, c(0), c(1), c(2), c(3))
    })?;
    Ok(Value::Bool(true))
}

fn fn_imagefill(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagefill";
    let (im, a) = ints(host, args, f, &["x", "y", "color"])?;
    let c = |i: usize| a[i] as c_int;
    gd(host, f, || unsafe {
        sys::gdImageFill(im, c(0), c(1), c(2))
    })?;
    Ok(Value::Bool(true))
}

fn fn_imagecolortransparent(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagecolortransparent";
    let im = img(host, args, 0, f, "image")?;
    if let Some(c) = lng_or_null(host, args, 1, f, "color")? {
        gd(host, f, || unsafe {
            sys::gdImageColorTransparent(im, c as c_int)
        })?;
    }
    Ok(Value::Int(unsafe { sys::rhp_gd_get_transparent(im) } as i64))
}

fn fn_imageinterlace(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imageinterlace";
    let im = img(host, args, 0, f, "image")?;
    match args.get(1).map(deref) {
        None | Some(Value::Null) => {}
        Some(_) => {
            let b = bln(host, args, 1, f, "enable", false)?;
            gd(host, f, || unsafe { sys::gdImageInterlace(im, b as c_int) })?;
        }
    }
    Ok(Value::Bool(unsafe { sys::rhp_gd_get_interlaced(im) } != 0))
}

/// php_imagepolygon: -1 open, 0 normal, 1 filled.
fn polygon(host: &mut dyn Host, args: &[Value], f: &str, filled: i8) -> Result<Value, Thrown> {
    let im = img(host, args, 0, f, "image")?;
    let points = arr(host, args, 1, f)?;
    let mut npoints = lng(host, args, 2, f, "num_points_or_color")?;
    let col = match lng_or_null(host, args, 3, f, "color")? {
        None => {
            let c = npoints;
            let n = points.len() as i64;
            if n % 2 != 0 {
                return Err(verr(f, 2, "points", "must have an even number of elements"));
            }
            npoints = n / 2;
            c
        }
        Some(c) => {
            crate::deprecated(host, f, "Using the $num_points parameter is deprecated")?;
            c
        }
    };
    let npoints = npoints as c_int;
    let mut col = col as c_int;
    let nelem = points.len() as c_int;
    if npoints < 3 {
        return Err(verr(
            f,
            3,
            "num_points_or_color",
            "must be greater than or equal to 3",
        ));
    }
    if nelem < npoints.wrapping_mul(2) {
        return Err(Thrown::error(
            "ValueError",
            format!(
                "Trying to use {npoints} points in array with only {} points",
                nelem / 2
            ),
        ));
    }
    let mut pts = vec![sys::GdPoint::default(); npoints as usize];
    for (i, p) in pts.iter_mut().enumerate() {
        if let Some(v) = points.get(&ArrayKey::Int(i as i64 * 2)) {
            p.x = elem_long(host, v)? as c_int;
        }
        if let Some(v) = points.get(&ArrayKey::Int(i as i64 * 2 + 1)) {
            p.y = elem_long(host, v)? as c_int;
        }
    }
    gd(host, f, || unsafe {
        if sys::rhp_gd_aa(im) != 0 {
            sys::gdImageSetAntiAliased(im, col);
            col = GD_ANTI_ALIASED;
        }
        match filled {
            -1 => sys::gdImageOpenPolygon(im, pts.as_mut_ptr(), npoints, col),
            0 => sys::gdImagePolygon(im, pts.as_mut_ptr(), npoints, col),
            _ => sys::gdImageFilledPolygon(im, pts.as_mut_ptr(), npoints, col),
        }
    })?;
    Ok(Value::Bool(true))
}

fn fn_imagepolygon(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    polygon(host, args, "imagepolygon", 0)
}
fn fn_imageopenpolygon(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    polygon(host, args, "imageopenpolygon", -1)
}
fn fn_imagefilledpolygon(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    polygon(host, args, "imagefilledpolygon", 1)
}

fn fn_imagesetclip(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagesetclip";
    let (im, a) = ints(host, args, f, &["x1", "y1", "x2", "y2"])?;
    let c = |i: usize| a[i] as c_int;
    gd(host, f, || unsafe {
        sys::gdImageSetClip(im, c(0), c(1), c(2), c(3))
    })?;
    Ok(Value::Bool(true))
}

fn fn_imagegetclip(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let im = img(host, args, 0, "imagegetclip", "image")?;
    let (mut x1, mut y1, mut x2, mut y2) = (0, 0, 0, 0);
    unsafe { sys::gdImageGetClip(im, &mut x1, &mut y1, &mut x2, &mut y2) };
    Ok(int_list(&[x1 as i64, y1 as i64, x2 as i64, y2 as i64]))
}

// ---------------------------------------------------------------------------
// FreeType text
// ---------------------------------------------------------------------------

/// php_imagettftext_common: `draw` (imagefttext) or the bounding box only
/// (imageftbbox).
fn ttftext(host: &mut dyn Host, args: &[Value], f: &str, draw: bool) -> Result<Value, Thrown> {
    let (im, base) = if draw {
        (img(host, args, 0, f, "image")?, 1)
    } else {
        (ptr::null_mut(), 0)
    };
    let ptsize = dbl(host, args, base, f, "size")?;
    let mut angle = dbl(host, args, base + 1, f, "angle")?;
    let (x, y, col, rest) = if draw {
        (
            lng(host, args, 3, f, "x")?,
            lng(host, args, 4, f, "y")?,
            lng(host, args, 5, f, "color")?,
            6,
        )
    } else {
        (0, 0, -1, 2)
    };
    let fontname = zpp::str_param(host, args, rest, f, "font_filename")?;
    let text_name = if draw { "text" } else { "string" };
    let text = zpp::str_param(host, args, rest + 1, f, text_name)?;
    let ext = match args.get(rest + 2) {
        Some(_) => Some(arr(host, args, rest + 2, f)?),
        None => None,
    };
    // gd.c names argument 2 for both forms: `$angle` for imageftbbox.
    let arg2 = if draw { "size" } else { "angle" };
    let lim = (i64::MIN as f64 / 64.0, i64::MAX as f64 / 64.0);
    if ptsize < lim.0 || ptsize > lim.1 {
        return Err(verr(
            f,
            2,
            arg2,
            &format!("must be between {} and {}", lim.0 as i64, lim.1 as i64),
        ));
    }
    if !ptsize.is_finite() {
        return Err(verr(f, 2, arg2, "must be finite"));
    }
    angle *= std::f64::consts::PI / 180.0;
    let mut linespace = None;
    if let Some(e) = &ext {
        if !e.is_packed() {
            for (k, v) in e.iter() {
                if let ArrayKey::Str(s) = k {
                    if s.as_bytes() == b"linespacing" {
                        linespace = Some(elem_double(host, v)?);
                    }
                }
            }
        }
    }
    // C strings: the font name and text end at a NUL.
    let cut = |s: &[u8]| -> CString {
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        CString::new(&s[..end]).unwrap_or_default()
    };
    let font = font_path(&fontname);
    let font = cut(&font);
    let text = cut(&text);
    let mut brect = [0 as c_int; 8];
    let err = gd(host, f, || unsafe {
        sys::rhp_gd_string_ft(
            im,
            brect.as_mut_ptr(),
            col as c_int,
            font.as_ptr(),
            ptsize,
            angle,
            x as c_int,
            y as c_int,
            text.as_ptr(),
            ext.is_some() as c_int,
            linespace.is_some() as c_int,
            linespace.unwrap_or(0.0),
        )
    })?;
    if !err.is_null() {
        let msg = c_bytes(err);
        crate::warn(host, f, &msg)?;
        return Ok(Value::Bool(false));
    }
    Ok(int_list(&brect.map(|b| b as i64)))
}

/// libgd opens a font path itself, relative to the process's directory: a
/// relative path is made absolute against the request's.
fn font_path(name: &[u8]) -> Vec<u8> {
    if name.contains(&b'/') && !name.starts_with(b"/") {
        crate::zend::virtual_cwd::absolute(name)
    } else {
        name.to_vec()
    }
}

fn fn_imageftbbox(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    ttftext(host, args, "imageftbbox", false)
}
fn fn_imagefttext(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    ttftext(host, args, "imagefttext", true)
}
fn fn_imagettfbbox(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    ttftext(host, args, "imagettfbbox", false)
}
fn fn_imagettftext(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    ttftext(host, args, "imagettftext", true)
}

// ---------------------------------------------------------------------------
// filters
// ---------------------------------------------------------------------------

/// A filter's own int argument: zpp of imagefilter's variadic `$args`,
/// whose errors name no parameter.
fn filter_long(host: &mut dyn Host, args: &[Value], i: usize, f: &str) -> Result<i64, Thrown> {
    let v = arg(args, i);
    filter_scalar(host, &v, i, f, "int")?;
    if matches!(v, Value::Null) {
        filter_null(host, i, f, "int")?;
        return Ok(0);
    }
    zpp::long_value(host, &v, i + 1, f, "", "int").map_err(|e| unname(e, f, i))
}

fn filter_double(host: &mut dyn Host, args: &[Value], i: usize, f: &str) -> Result<f64, Thrown> {
    let v = arg(args, i);
    filter_scalar(host, &v, i, f, "float")?;
    if matches!(v, Value::Null) {
        filter_null(host, i, f, "float")?;
        return Ok(0.0);
    }
    zpp::double_value(host, &v, i + 1, f, "").map_err(|e| unname(e, f, i))
}

fn filter_bool(host: &mut dyn Host, args: &[Value], i: usize, f: &str) -> Result<bool, Thrown> {
    let v = arg(args, i);
    filter_scalar(host, &v, i, f, "bool")?;
    match v {
        Value::Bool(b) => Ok(b),
        Value::Null => {
            filter_null(host, i, f, "bool")?;
            Ok(false)
        }
        _ if host.caller_strict_types() => Err(filter_type(host, &v, i, f, "bool")),
        v => Ok(rhp_values::conv::to_bool(&v)),
    }
}

fn filter_type(host: &dyn Host, v: &Value, i: usize, f: &str, ty: &str) -> Thrown {
    Thrown::type_error(format!(
        "{f}(): Argument #{} must be of type {ty}, {} given",
        i + 1,
        zpp::zval_value_name(host, v)
    ))
}

fn filter_scalar(host: &dyn Host, v: &Value, i: usize, f: &str, ty: &str) -> Result<(), Thrown> {
    match v {
        Value::Array(_) | Value::Object(_) | Value::Closure(_) | Value::Resource(_) => {
            Err(filter_type(host, v, i, f, ty))
        }
        _ => Ok(()),
    }
}

fn filter_null(host: &mut dyn Host, i: usize, f: &str, ty: &str) -> Result<(), Thrown> {
    if host.caller_strict_types() {
        return Err(filter_type(host, &Value::Null, i, f, ty));
    }
    host.emit_diagnostic(
        crate::E_DEPRECATED,
        format!(
            "{f}(): Passing null to parameter #{} of type {ty} is deprecated",
            i + 1
        )
        .as_bytes(),
    )
}

/// The parameter-name part ` ($)` of a zpp message for an unnamed
/// argument, dropped.
fn unname(e: Thrown, _f: &str, _i: usize) -> Thrown {
    match e {
        Thrown::Error { class, message } => Thrown::Error {
            class,
            message: message.replacen(" ($)", "", 1),
        },
        e => e,
    }
}

const IMAGE_FILTER_MAX: i64 = 12;
const IMAGE_FILTER_MAX_ARGS: usize = 6;

fn fn_imagefilter(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagefilter";
    if args.len() < 2 || args.len() > IMAGE_FILTER_MAX_ARGS {
        return Err(Thrown::error(
            "ArgumentCountError",
            format!("Wrong parameter count for {f}()"),
        ));
    }
    let im = img(host, args, 0, f, "image")?;
    let kind = lng(host, args, 1, f, "filter")?;
    if !(0..=IMAGE_FILTER_MAX).contains(&kind) {
        return Err(verr(
            f,
            2,
            "filter",
            "must be one of the IMG_FILTER_* filter constants",
        ));
    }
    let r = match kind {
        0 | 1 | 5 | 6 | 7 | 8 | 9 => {
            gd(host, f, || unsafe {
                match kind {
                    0 => sys::gdImageNegate(im),
                    1 => sys::gdImageGrayScale(im),
                    5 => sys::gdImageEdgeDetectQuick(im),
                    6 => sys::gdImageEmboss(im),
                    7 => sys::gdImageGaussianBlur(im),
                    8 => sys::gdImageSelectiveBlur(im),
                    _ => sys::gdImageMeanRemoval(im),
                }
            })? == 1
        }
        2 | 3 => {
            exact(f, args.len(), 3)?;
            let v = filter_long(host, args, 2, f)?;
            gd(host, f, || unsafe {
                if kind == 2 {
                    sys::gdImageBrightness(im, v as c_int)
                } else {
                    sys::gdImageContrast(im, v as c_int as f64)
                }
            })? == 1
        }
        4 => {
            between(f, args.len(), 5, 6)?;
            let r = filter_long(host, args, 2, f)?;
            let g = filter_long(host, args, 3, f)?;
            let b = filter_long(host, args, 4, f)?;
            let a = if args.len() > 5 {
                filter_long(host, args, 5, f)?
            } else {
                0
            };
            gd(host, f, || unsafe {
                sys::gdImageColor(im, r as c_int, g as c_int, b as c_int, a as c_int)
            })? == 1
        }
        10 => {
            exact(f, args.len(), 3)?;
            let w = filter_double(host, args, 2, f)?;
            gd(host, f, || unsafe { sys::gdImageSmooth(im, w as f32) })? == 1
        }
        11 => {
            between(f, args.len(), 3, 4)?;
            let block = filter_long(host, args, 2, f)?;
            let mode = if args.len() > 3 {
                filter_bool(host, args, 3, f)?
            } else {
                false
            };
            gd(host, f, || unsafe {
                sys::gdImagePixelate(im, block as c_int, mode as u32)
            })? != 0
        }
        _ => {
            between(f, args.len(), 4, 5)?;
            let sub = filter_long(host, args, 2, f)?;
            let plus = filter_long(host, args, 3, f)?;
            let colors = match args.get(4) {
                Some(_) => match arg(args, 4) {
                    Value::Array(a) => Some(a),
                    v => {
                        return Err(filter_type(host, &v, 4, f, "array"));
                    }
                },
                None => None,
            };
            if !(0..=INT_MAX).contains(&sub) {
                return Err(Thrown::error(
                    "ValueError",
                    format!("{f}(): Argument #3 must be between 0 and {INT_MAX}"),
                ));
            }
            if !(0..=INT_MAX).contains(&plus) {
                return Err(Thrown::error(
                    "ValueError",
                    format!("{f}(): Argument #4 must be between 0 and {INT_MAX}"),
                ));
            }
            match colors.filter(|c| !c.is_empty()) {
                Some(c) => {
                    let mut v = Vec::with_capacity(c.len());
                    for (_, x) in c.iter() {
                        v.push(elem_long(host, x)? as c_int);
                    }
                    gd(host, f, || unsafe {
                        sys::gdImageScatterColor(
                            im,
                            sub as c_int,
                            plus as c_int,
                            v.as_mut_ptr(),
                            v.len() as u32,
                        )
                    })? != 0
                }
                None => {
                    gd(host, f, || unsafe {
                        sys::gdImageScatter(im, sub as c_int, plus as c_int)
                    })? != 0
                }
            }
        }
    };
    Ok(Value::Bool(r))
}

/// ZEND_PARSE_PARAMETERS_START(n, n)'s count error.
fn exact(f: &str, got: usize, n: usize) -> Result<(), Thrown> {
    if got != n {
        return Err(Thrown::error(
            "ArgumentCountError",
            format!(
                "{f}() expects exactly {n} argument{}, {got} given",
                if n == 1 { "" } else { "s" }
            ),
        ));
    }
    Ok(())
}

fn between(f: &str, got: usize, lo: usize, hi: usize) -> Result<(), Thrown> {
    if got < lo {
        return Err(Thrown::error(
            "ArgumentCountError",
            format!("{f}() expects at least {lo} arguments, {got} given"),
        ));
    }
    if got > hi {
        return Err(Thrown::error(
            "ArgumentCountError",
            format!("{f}() expects at most {hi} arguments, {got} given"),
        ));
    }
    Ok(())
}

fn fn_imageconvolution(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imageconvolution";
    let im = img(host, args, 0, f, "image")?;
    let m = arr(host, args, 1, f)?;
    let div = dbl(host, args, 2, f, "divisor")?;
    let offset = dbl(host, args, 3, f, "offset")?;
    if m.len() != 3 {
        return Err(verr(f, 2, "matrix", "must be a 3x3 array"));
    }
    let mut matrix = [[0f32; 3]; 3];
    for (i, row) in matrix.iter_mut().enumerate() {
        let Some(Value::Array(r)) = m.get(&ArrayKey::Int(i as i64)).map(deref) else {
            continue;
        };
        if r.len() != 3 {
            return Err(verr(
                f,
                2,
                "matrix",
                &format!(
                    "must be a 3x3 array, matrix[{i}] only has {} elements",
                    r.len()
                ),
            ));
        }
        for (j, cell) in row.iter_mut().enumerate() {
            match r.get(&ArrayKey::Int(j as i64)) {
                Some(v) => *cell = elem_double(host, v)? as f32,
                None => {
                    return Err(verr(
                        f,
                        2,
                        "matrix",
                        &format!(
                            "must be a 3x3 array, matrix[{i}][{j}] cannot be found (missing integer key)"
                        ),
                    ))
                }
            }
        }
    }
    if !div.is_finite() {
        return Err(verr(f, 3, "divisor", "must be finite"));
    }
    let div_f = div as f32;
    if div_f == 0.0 {
        return Err(verr(f, 3, "divisor", "must not be 0"));
    }
    if !offset.is_finite() {
        return Err(verr(f, 4, "offset", "must be finite"));
    }
    let r = gd(host, f, || unsafe {
        sys::gdImageConvolution(im, matrix.as_mut_ptr(), div_f, offset as f32)
    })?;
    Ok(Value::Bool(r != 0))
}

fn fn_imageflip(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imageflip";
    let (im, a) = ints(host, args, f, &["mode"])?;
    let g: unsafe extern "C" fn(Im) = match a[0] {
        FLIP_VERTICAL => sys::gdImageFlipVertical,
        FLIP_HORIZONTAL => sys::gdImageFlipHorizontal,
        FLIP_BOTH => sys::gdImageFlipBoth,
        _ => {
            return Err(verr(
                f,
                2,
                "mode",
                "must be one of IMG_FLIP_VERTICAL, IMG_FLIP_HORIZONTAL, or IMG_FLIP_BOTH",
            ))
        }
    };
    gd(host, f, || unsafe { g(im) })?;
    Ok(Value::Bool(true))
}

fn fn_imageantialias(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imageantialias";
    let im = img(host, args, 0, f, "image")?;
    let b = bln(host, args, 1, f, "enable", false)?;
    unsafe {
        if sys::rhp_gd_truecolor(im) != 0 {
            sys::rhp_gd_set_aa(im, b as c_int);
        }
    }
    Ok(Value::Bool(true))
}

/// The `x`, `y`, `width` and `height` keys of a rectangle argument.
fn rect_arg(
    host: &mut dyn Host,
    r: &PhpArray,
    f: &str,
    n: usize,
    p: &str,
) -> Result<sys::GdRect, Thrown> {
    let mut out = [0 as c_int; 4];
    for (k, (key, article)) in [("x", "an"), ("y", "a"), ("width", "a"), ("height", "a")]
        .iter()
        .enumerate()
    {
        match r.get(&akey(key)) {
            Some(v) => out[k] = elem_long(host, v)? as c_int,
            None => {
                return Err(verr(f, n, p, &format!("must have {article} \"{key}\" key")));
            }
        }
    }
    Ok(sys::GdRect {
        x: out[0],
        y: out[1],
        width: out[2],
        height: out[3],
    })
}

fn fn_imagecrop(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagecrop";
    let im = img(host, args, 0, f, "image")?;
    let r = arr(host, args, 1, f)?;
    let rect = rect_arg(host, &r, f, 2, "rectangle")?;
    let (x, y, w, h) = (rect.x, rect.y, rect.width, rect.height);
    let e = |m: &str| Err(verr(f, 2, "rectangle", m));
    if w > 0 && x > i32::MAX - w {
        return e("overflow with \"x\" and \"width\" keys");
    }
    if w < 0 && x < i32::MIN - w {
        return e("underflow with \"x\" and \"width\" keys");
    }
    if h > 0 && y > i32::MAX - h {
        return e("overflow with \"y\" and \"height\" keys");
    }
    if h < 0 && y < i32::MIN - h {
        return e("underflow with \"y\" and \"height\" keys");
    }
    let r = gd(host, f, || unsafe { sys::gdImageCrop(im, &rect) })?;
    image_or_false(host, r)
}

fn fn_imagecropauto(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagecropauto";
    let im = img(host, args, 0, f, "image")?;
    let mode = lng_or(host, args, 1, f, "mode", GD_CROP_DEFAULT)?;
    let threshold = if args.len() > 2 {
        dbl(host, args, 2, f, "threshold")?
    } else {
        0.5
    };
    let color = lng_or(host, args, 3, f, "color", -1)?;
    let r = match mode {
        GD_CROP_DEFAULT..=GD_CROP_SIDES => {
            gd(host, f, || unsafe { sys::gdImageCropAuto(im, mode as u32) })?
        }
        GD_CROP_THRESHOLD => {
            if color < 0
                || (unsafe { sys::rhp_gd_truecolor(im) } == 0
                    && color >= unsafe { sys::rhp_gd_colors_total(im) } as i64)
            {
                return Err(verr(
                    f,
                    4,
                    "color",
                    "must be greater than or equal to 0 when using the threshold mode",
                ));
            }
            gd(host, f, || unsafe {
                sys::gdImageCropThreshold(im, color as u32, threshold as f32)
            })?
        }
        _ => return Err(verr(f, 2, "mode", "must be a valid mode")),
    };
    image_or_false(host, r)
}

fn fn_imagescale(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagescale";
    let im = img(host, args, 0, f, "image")?;
    let mut w = lng(host, args, 1, f, "width")?;
    let mut h = lng_or(host, args, 2, f, "height", -1)?;
    let m = lng_or(host, args, 3, f, "mode", GD_BILINEAR_FIXED)?;
    if m < gd_const("=GD_DEFAULT") || m >= gd_const("=GD_METHOD_COUNT") {
        return Err(verr(f, 4, "mode", "must be one of the GD_* constants"));
    }
    if h < 0 && w < 0 {
        return Err(Thrown::error(
            "ValueError",
            "Argument #2 ($width) and argument #3 ($height) cannot be both negative",
        ));
    }
    if h < 0 || w < 0 {
        let sx = unsafe { sys::rhp_gd_sx(im) } as i64;
        let sy = unsafe { sys::rhp_gd_sy(im) } as i64;
        if sx != 0 && h < 0 {
            if w > i64::MAX / sy {
                return Err(verr(
                    f,
                    2,
                    "width",
                    &format!("must be less than or equal to {}", i64::MAX / sy),
                ));
            }
            h = w * sy / sx;
        }
        if sy != 0 && w < 0 {
            if h > i64::MAX / sx {
                return Err(verr(
                    f,
                    3,
                    "height",
                    &format!("must be less than or equal to {}", i64::MAX / sx),
                ));
            }
            w = h * sx / sy;
        }
    }
    if w <= 0 || w > INT_MAX {
        return Err(verr(
            f,
            2,
            "width",
            &format!("must be between 1 and {INT_MAX}"),
        ));
    }
    if h <= 0 || h > INT_MAX {
        return Err(verr(
            f,
            3,
            "height",
            &format!("must be between 1 and {INT_MAX}"),
        ));
    }
    let r = gd(host, f, || unsafe {
        let old = sys::rhp_gd_interpolation_id(im);
        let mut r = ptr::null_mut();
        if sys::gdImageSetInterpolationMethod(im, m as c_int) != 0 {
            r = sys::gdImageScale(im, w as u32, h as u32);
        }
        sys::gdImageSetInterpolationMethod(im, old);
        r
    })?;
    image_or_false(host, r)
}

/// An affine matrix element: ints, floats and strings as numbers.
fn matrix_elem(host: &mut dyn Host, v: &Value) -> Option<Result<f64, Thrown>> {
    match deref(v) {
        Value::Int(_) | Value::Float(_) | Value::Str(_) => Some(elem_double(host, v)),
        _ => None,
    }
}

fn fn_imageaffine(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imageaffine";
    let im = img(host, args, 0, f, "image")?;
    let z = arr(host, args, 1, f)?;
    let clip = match arg(args, 2) {
        Value::Null => None,
        _ => Some(arr(host, args, 2, f)?),
    };
    if z.len() != 6 {
        return Err(verr(f, 2, "affine", "must have 6 elements"));
    }
    let mut affine = [0f64; 6];
    for (i, a) in affine.iter_mut().enumerate() {
        let Some(v) = z.get(&ArrayKey::Int(i as i64)) else {
            continue;
        };
        match matrix_elem(host, v) {
            Some(x) => {
                *a = x?;
                if *a < INT_MIN as f64 || *a > INT_MAX as f64 {
                    return Err(verr(
                        f,
                        2,
                        "affine",
                        &format!("element {i} must be between {INT_MIN} and {INT_MAX}"),
                    ));
                }
            }
            None => {
                return Err(Thrown::type_error(format!(
                    "{f}(): Argument #3 ($clip) contains invalid type for element {i}"
                )))
            }
        }
    }
    let mut rect = match &clip {
        Some(c) => Some(rect_arg(host, c, f, 3, "clip")?),
        None => None,
    };
    let mut dst: Im = ptr::null_mut();
    let ok = gd(host, f, || unsafe {
        let p = rect.as_mut().map_or(ptr::null_mut(), |r| r as *mut _);
        sys::gdTransformAffineGetImage(&mut dst, im, p, affine.as_ptr())
    })?;
    if ok != 1 {
        return Ok(Value::Bool(false));
    }
    image_or_false(host, dst)
}

fn fn_imageaffinematrixget(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imageaffinematrixget";
    let kind = lng(host, args, 0, f, "type")?;
    let options = arg(args, 1);
    let mut affine = [0f64; 6];
    let res = match kind {
        0 | 1 => {
            let Value::Array(o) = &options else {
                return Err(Thrown::type_error(format!(
                    "{f}(): Argument #2 ($options) must be of type array when using translate or scale"
                )));
            };
            let x = match o.get(&akey("x")) {
                Some(v) => elem_double(host, v)?,
                None => return Err(verr(f, 2, "options", "must have an \"x\" key")),
            };
            let y = match o.get(&akey("y")) {
                Some(v) => elem_double(host, v)?,
                None => return Err(verr(f, 2, "options", "must have a \"y\" key")),
            };
            unsafe {
                if kind == 0 {
                    sys::gdAffineTranslate(affine.as_mut_ptr(), x, y)
                } else {
                    sys::gdAffineScale(affine.as_mut_ptr(), x, y)
                }
            }
        }
        2..=4 => {
            let a = elem_double(host, &options)?;
            unsafe {
                match kind {
                    3 => sys::gdAffineShearHorizontal(affine.as_mut_ptr(), a),
                    4 => sys::gdAffineShearVertical(affine.as_mut_ptr(), a),
                    _ => sys::gdAffineRotate(affine.as_mut_ptr(), a),
                }
            }
        }
        _ => return Err(verr(f, 1, "type", "must be a valid element type")),
    };
    if res == 0 {
        return Ok(Value::Bool(false));
    }
    let mut r = PhpArray::new();
    for a in affine {
        r.push(Value::Float(a));
    }
    Ok(Value::Array(r))
}

fn fn_imageaffinematrixconcat(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imageaffinematrixconcat";
    let z1 = arr(host, args, 0, f)?;
    let z2 = arr(host, args, 1, f)?;
    if z1.len() != 6 {
        return Err(verr(f, 1, "matrix1", "must have 6 elements"));
    }
    if z2.len() != 6 {
        return Err(verr(f, 2, "matrix2", "must have 6 elements"));
    }
    let mut m1 = [0f64; 6];
    let mut m2 = [0f64; 6];
    for i in 0..6 {
        for (n, z, m, p) in [(1, &z1, &mut m1, "matrix1"), (2, &z2, &mut m2, "matrix2")] {
            if let Some(v) = z.get(&ArrayKey::Int(i as i64)) {
                match matrix_elem(host, v) {
                    Some(x) => m[i] = x?,
                    None => {
                        return Err(Thrown::type_error(format!(
                            "{f}(): Argument #{n} (${p}) contains invalid type for element {i}"
                        )))
                    }
                }
            }
        }
    }
    let mut mr = [0f64; 6];
    if unsafe { sys::gdAffineConcat(mr.as_mut_ptr(), m1.as_ptr(), m2.as_ptr()) } != 1 {
        return Ok(Value::Bool(false));
    }
    let mut r = PhpArray::new();
    for a in mr {
        r.push(Value::Float(a));
    }
    Ok(Value::Array(r))
}

fn fn_imagegetinterpolation(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let im = img(host, args, 0, "imagegetinterpolation", "image")?;
    Ok(Value::Int(
        unsafe { sys::gdImageGetInterpolationMethod(im) } as i64,
    ))
}

fn fn_imagesetinterpolation(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imagesetinterpolation";
    let im = img(host, args, 0, f, "image")?;
    let mut m = lng_or(host, args, 1, f, "method", GD_BILINEAR_FIXED)?;
    if m == -1 {
        m = GD_BILINEAR_FIXED;
    }
    let r = gd(host, f, || unsafe {
        sys::gdImageSetInterpolationMethod(im, m as c_int)
    })?;
    Ok(Value::Bool(r != 0))
}

fn fn_imageresolution(host: &mut dyn Host, args: &[Value]) -> Result<Value, Thrown> {
    let f = "imageresolution";
    let im = img(host, args, 0, f, "image")?;
    let rx = lng_or_null(host, args, 1, f, "resolution_x")?;
    let ry = lng_or_null(host, args, 2, f, "resolution_y")?;
    let check = |v: i64, n: usize, p: &str| {
        if v < 0 || v > u32::MAX as i64 {
            Err(verr(
                f,
                n,
                p,
                &format!("must be between 0 and {}", u32::MAX),
            ))
        } else {
            Ok(())
        }
    };
    let set = match (rx, ry) {
        (Some(x), Some(y)) => {
            check(x, 2, "resolution_x")?;
            check(y, 3, "resolution_y")?;
            Some((x, y))
        }
        (Some(x), None) => {
            check(x, 2, "resolution_x")?;
            Some((x, x))
        }
        (None, Some(y)) => {
            check(y, 3, "resolution_y")?;
            Some((y, y))
        }
        (None, None) => None,
    };
    if let Some((x, y)) = set {
        unsafe { sys::gdImageSetResolution(im, x as u32, y as u32) };
        return Ok(Value::Bool(true));
    }
    unsafe {
        Ok(int_list(&[
            sys::rhp_gd_res_x(im) as i64,
            sys::rhp_gd_res_y(im) as i64,
        ]))
    }
}

// ---------------------------------------------------------------------------
// registration
// ---------------------------------------------------------------------------

pub fn register(m: &mut crate::Registry) {
    // The format functions php's build has (HAVE_GD_*).
    let optional: [(i32, &[(&'static str, crate::NativeFn)]); 5] = [
        (
            HAVE_AVIF,
            &[
                ("imagecreatefromavif", fn_imagecreatefromavif),
                ("imageavif", fn_imageavif),
            ],
        ),
        (
            HAVE_JPG,
            &[
                ("imagecreatefromjpeg", fn_imagecreatefromjpeg),
                ("imagejpeg", fn_imagejpeg),
            ],
        ),
        (
            HAVE_PNG,
            &[
                ("imagecreatefrompng", fn_imagecreatefrompng),
                ("imagepng", fn_imagepng),
            ],
        ),
        (
            HAVE_WEBP,
            &[
                ("imagecreatefromwebp", fn_imagecreatefromwebp),
                ("imagewebp", fn_imagewebp),
            ],
        ),
        (HAVE_XPM, &[("imagecreatefromxpm", fn_imagecreatefromxpm)]),
    ];
    for (bit, fns) in optional {
        if have(bit) {
            crate::register(m, fns);
        }
    }
    crate::register(
        m,
        &[
            ("gd_info", fn_gd_info),
            ("imageloadfont", fn_imageloadfont),
            ("imagesetstyle", fn_imagesetstyle),
            ("imagecreatetruecolor", fn_imagecreatetruecolor),
            ("imageistruecolor", fn_imageistruecolor),
            ("imagetruecolortopalette", fn_imagetruecolortopalette),
            ("imagepalettetotruecolor", fn_imagepalettetotruecolor),
            ("imagecolormatch", fn_imagecolormatch),
            ("imagesetthickness", fn_imagesetthickness),
            ("imagefilledellipse", fn_imagefilledellipse),
            ("imagefilledarc", fn_imagefilledarc),
            ("imagealphablending", fn_imagealphablending),
            ("imagesavealpha", fn_imagesavealpha),
            ("imagelayereffect", fn_imagelayereffect),
            ("imagecolorallocatealpha", fn_imagecolorallocatealpha),
            ("imagecolorresolvealpha", fn_imagecolorresolvealpha),
            ("imagecolorclosestalpha", fn_imagecolorclosestalpha),
            ("imagecolorexactalpha", fn_imagecolorexactalpha),
            ("imagecopyresampled", fn_imagecopyresampled),
            ("imagerotate", fn_imagerotate),
            ("imagesettile", fn_imagesettile),
            ("imagesetbrush", fn_imagesetbrush),
            ("imagecreate", fn_imagecreate),
            ("imagetypes", fn_imagetypes),
            ("imagecreatefromstring", fn_imagecreatefromstring),
            ("imagecreatefromgif", fn_imagecreatefromgif),
            ("imagecreatefromxbm", fn_imagecreatefromxbm),
            ("imagecreatefromwbmp", fn_imagecreatefromwbmp),
            ("imagecreatefromgd", fn_imagecreatefromgd),
            ("imagecreatefromgd2", fn_imagecreatefromgd2),
            ("imagecreatefromgd2part", fn_imagecreatefromgd2part),
            ("imagecreatefrombmp", fn_imagecreatefrombmp),
            ("imagecreatefromtga", fn_imagecreatefromtga),
            ("imagexbm", fn_imagexbm),
            ("imagegif", fn_imagegif),
            ("imagewbmp", fn_imagewbmp),
            ("imagegd", fn_imagegd),
            ("imagegd2", fn_imagegd2),
            ("imagebmp", fn_imagebmp),
            ("imagedestroy", fn_imagedestroy),
            ("imagecolorallocate", fn_imagecolorallocate),
            ("imagepalettecopy", fn_imagepalettecopy),
            ("imagecolorat", fn_imagecolorat),
            ("imagecolorclosest", fn_imagecolorclosest),
            ("imagecolorclosesthwb", fn_imagecolorclosesthwb),
            ("imagecolordeallocate", fn_imagecolordeallocate),
            ("imagecolorresolve", fn_imagecolorresolve),
            ("imagecolorexact", fn_imagecolorexact),
            ("imagecolorset", fn_imagecolorset),
            ("imagecolorsforindex", fn_imagecolorsforindex),
            ("imagegammacorrect", fn_imagegammacorrect),
            ("imagesetpixel", fn_imagesetpixel),
            ("imageline", fn_imageline),
            ("imagedashedline", fn_imagedashedline),
            ("imagerectangle", fn_imagerectangle),
            ("imagefilledrectangle", fn_imagefilledrectangle),
            ("imagearc", fn_imagearc),
            ("imageellipse", fn_imageellipse),
            ("imagefilltoborder", fn_imagefilltoborder),
            ("imagefill", fn_imagefill),
            ("imagecolorstotal", fn_imagecolorstotal),
            ("imagecolortransparent", fn_imagecolortransparent),
            ("imageinterlace", fn_imageinterlace),
            ("imagepolygon", fn_imagepolygon),
            ("imageopenpolygon", fn_imageopenpolygon),
            ("imagefilledpolygon", fn_imagefilledpolygon),
            ("imagefontwidth", fn_imagefontwidth),
            ("imagefontheight", fn_imagefontheight),
            ("imagechar", fn_imagechar),
            ("imagecharup", fn_imagecharup),
            ("imagestring", fn_imagestring),
            ("imagestringup", fn_imagestringup),
            ("imagecopy", fn_imagecopy),
            ("imagecopymerge", fn_imagecopymerge),
            ("imagecopymergegray", fn_imagecopymergegray),
            ("imagecopyresized", fn_imagecopyresized),
            ("imagesx", fn_imagesx),
            ("imagesy", fn_imagesy),
            ("imagesetclip", fn_imagesetclip),
            ("imagegetclip", fn_imagegetclip),
            ("imageftbbox", fn_imageftbbox),
            ("imagefttext", fn_imagefttext),
            ("imagettfbbox", fn_imagettfbbox),
            ("imagettftext", fn_imagettftext),
            ("imagefilter", fn_imagefilter),
            ("imageconvolution", fn_imageconvolution),
            ("imageflip", fn_imageflip),
            ("imageantialias", fn_imageantialias),
            ("imagecrop", fn_imagecrop),
            ("imagecropauto", fn_imagecropauto),
            ("imagescale", fn_imagescale),
            ("imageaffine", fn_imageaffine),
            ("imageaffinematrixget", fn_imageaffinematrixget),
            ("imageaffinematrixconcat", fn_imageaffinematrixconcat),
            ("imagegetinterpolation", fn_imagegetinterpolation),
            ("imagesetinterpolation", fn_imagesetinterpolation),
            ("imageresolution", fn_imageresolution),
        ],
    );
}
