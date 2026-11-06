//! libavifinfo (php-src ext/standard/libavifinfo), which getimagesize()
//! uses to identify AVIF/HEIF files and read their features. The C pulls
//! bytes through two callbacks; [`AvifStream`] is what they read from.

use std::ffi::{c_int, c_void};

/// `AVIFINFO_MAX_NUM_READ_BYTES`: the most one read callback asks for.
const MAX_READ: usize = 64;

/// The stream the C reads, as php's `php_avif_stream_read`/`_skip` see a
/// php_stream.
pub trait AvifStream {
    /// Exactly `buf.len()` bytes into `buf`; false when fewer are left.
    fn read_exact(&mut self, buf: &mut [u8]) -> bool;
    /// Moves on `n` bytes; false when the seek fails.
    fn skip(&mut self, n: usize) -> bool;
}

/// `AvifInfoFeatures`, the fields php reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Features {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u32,
    pub num_channels: u32,
}

#[repr(C)]
#[derive(Default)]
struct RawFeatures {
    width: u32,
    height: u32,
    bit_depth: u32,
    num_channels: u32,
    has_gainmap: u8,
    gainmap_item_id: u8,
    primary_item_id_location: u64,
    primary_item_id_bytes: u8,
}

type ReadFn = unsafe extern "C" fn(*mut c_void, usize) -> *const u8;
type SkipFn = unsafe extern "C" fn(*mut c_void, usize);

// glue/prefix_avifinfo.h gives the C symbols an rhp_ prefix.
extern "C" {
    fn rhp_AvifInfoIdentifyStream(stream: *mut c_void, read: ReadFn, skip: SkipFn) -> c_int;
    fn rhp_AvifInfoGetFeaturesStream(
        stream: *mut c_void,
        read: ReadFn,
        skip: SkipFn,
        features: *mut RawFeatures,
    ) -> c_int;
}

/// `kAvifInfoOk`.
const OK: c_int = 0;

/// php's `struct php_avif_stream`: the stream (gone after a failed call, so
/// later calls fail too) and the buffer a read returns.
struct Ctx<'a> {
    stream: Option<&'a mut dyn AvifStream>,
    buf: [u8; MAX_READ],
}

unsafe extern "C" fn read_cb(ctx: *mut c_void, n: usize) -> *const u8 {
    // SAFETY: ctx is the &mut Ctx `with_ctx` passed, alive for the C call.
    let ctx = unsafe { &mut *(ctx as *mut Ctx) };
    let Some(s) = ctx.stream.as_deref_mut() else {
        return std::ptr::null();
    };
    if n > MAX_READ || !s.read_exact(&mut ctx.buf[..n]) {
        ctx.stream = None;
        return std::ptr::null();
    }
    ctx.buf.as_ptr()
}

unsafe extern "C" fn skip_cb(ctx: *mut c_void, n: usize) {
    // SAFETY: as in read_cb.
    let ctx = unsafe { &mut *(ctx as *mut Ctx) };
    if let Some(s) = ctx.stream.as_deref_mut() {
        if !s.skip(n) {
            ctx.stream = None;
        }
    }
}

/// `AvifInfoIdentifyStream(...) == kAvifInfoOk`.
pub fn identify(stream: &mut dyn AvifStream) -> bool {
    let mut ctx = Ctx {
        stream: Some(stream),
        buf: [0; MAX_READ],
    };
    // SAFETY: the callbacks only use ctx, which outlives the call.
    unsafe {
        rhp_AvifInfoIdentifyStream(&mut ctx as *mut Ctx as *mut c_void, read_cb, skip_cb) == OK
    }
}

/// `AvifInfoGetFeaturesStream`: the features when it returns kAvifInfoOk.
pub fn features(stream: &mut dyn AvifStream) -> Option<Features> {
    let mut ctx = Ctx {
        stream: Some(stream),
        buf: [0; MAX_READ],
    };
    let mut f = RawFeatures::default();
    // SAFETY: as in identify; f is a valid AvifInfoFeatures.
    let st = unsafe {
        rhp_AvifInfoGetFeaturesStream(
            &mut ctx as *mut Ctx as *mut c_void,
            read_cb,
            skip_cb,
            &mut f,
        )
    };
    (st == OK).then_some(Features {
        width: f.width,
        height: f.height,
        bit_depth: f.bit_depth,
        num_channels: f.num_channels,
    })
}
