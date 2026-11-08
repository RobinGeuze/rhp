//! The libgd API ext/gd calls (gd.h, gd_io.h), and the accessors and
//! constants of `shim.c`.

#![allow(non_snake_case)]

use std::ffi::{c_char, c_double, c_float, c_int, c_long, c_uint, c_void};

/// `gdImage` (opaque: its fields go through shim.c).
#[repr(C)]
pub struct GdImage {
    _p: [u8; 0],
}
pub type Im = *mut GdImage;

/// `gdFont`.
#[repr(C)]
pub struct GdFont {
    pub nchars: c_int,
    pub offset: c_int,
    pub w: c_int,
    pub h: c_int,
    pub data: *mut c_char,
}

/// `gdIOCtx`.
#[repr(C)]
pub struct GdIoCtx {
    pub get_c: Option<unsafe extern "C" fn(*mut GdIoCtx) -> c_int>,
    pub get_buf: Option<unsafe extern "C" fn(*mut GdIoCtx, *mut c_void, c_int) -> c_int>,
    pub put_c: Option<unsafe extern "C" fn(*mut GdIoCtx, c_int)>,
    pub put_buf: Option<unsafe extern "C" fn(*mut GdIoCtx, *const c_void, c_int) -> c_int>,
    pub seek: Option<unsafe extern "C" fn(*mut GdIoCtx, c_int) -> c_int>,
    pub tell: Option<unsafe extern "C" fn(*mut GdIoCtx) -> c_long>,
    pub gd_free: Option<unsafe extern "C" fn(*mut GdIoCtx)>,
    pub data: *mut c_void,
}

/// `gdPoint`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GdPoint {
    pub x: c_int,
    pub y: c_int,
}

/// `gdRect`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GdRect {
    pub x: c_int,
    pub y: c_int,
    pub width: c_int,
    pub height: c_int,
}

#[repr(C)]
pub struct Const {
    pub name: *const c_char,
    pub value: c_long,
}

pub type CtxReader = unsafe extern "C" fn(*mut GdIoCtx) -> Im;

extern "C" {
    // shim.c
    pub fn rhp_gd_init();
    pub fn rhp_gd_features() -> c_int;
    pub fn rhp_gd_from_xpm(filename: *const c_char) -> Im;
    #[cfg(test)]
    pub fn rhp_gd_layout(which: c_int) -> usize;
    pub fn rhp_gd_level(which: c_int) -> c_int;
    pub fn rhp_gd_sx(im: Im) -> c_int;
    pub fn rhp_gd_sy(im: Im) -> c_int;
    pub fn rhp_gd_truecolor(im: Im) -> c_int;
    pub fn rhp_gd_colors_total(im: Im) -> c_int;
    pub fn rhp_gd_red(im: Im, c: c_int) -> c_int;
    pub fn rhp_gd_green(im: Im, c: c_int) -> c_int;
    pub fn rhp_gd_blue(im: Im, c: c_int) -> c_int;
    pub fn rhp_gd_alpha(im: Im, c: c_int) -> c_int;
    pub fn rhp_gd_get_transparent(im: Im) -> c_int;
    pub fn rhp_gd_get_interlaced(im: Im) -> c_int;
    pub fn rhp_gd_aa(im: Im) -> c_int;
    pub fn rhp_gd_set_aa(im: Im, aa: c_int);
    pub fn rhp_gd_interpolation_id(im: Im) -> c_int;
    pub fn rhp_gd_res_x(im: Im) -> c_uint;
    pub fn rhp_gd_res_y(im: Im) -> c_uint;
    pub fn rhp_gd_color_at(im: Im, x: c_long, y: c_long, out: *mut c_long) -> c_int;
    pub fn rhp_gd_set_palette(im: Im, c: c_int, r: c_int, g: c_int, b: c_int, a: c_int);
    pub fn rhp_gd_set_rgb(im: Im, c: c_int, r: c_int, g: c_int, b: c_int);
    pub fn rhp_gd_builtin_font(n: c_int) -> *mut GdFont;
    pub fn rhp_gd_string_ft(
        im: Im,
        brect: *mut c_int,
        fg: c_int,
        font: *const c_char,
        ptsize: c_double,
        angle: c_double,
        x: c_int,
        y: c_int,
        s: *const c_char,
        ext: c_int,
        linespace: c_int,
        linespacing: c_double,
    ) -> *mut c_char;
    pub fn rhp_gd_from_xbm(data: *const c_void, len: usize) -> Im;
    pub fn rhp_gd_consts() -> *const Const;
    pub fn rhp_gd_version_string() -> *const c_char;
    pub fn rhp_gd_extra_version() -> *const c_char;

    // gd.h
    pub fn gdVersionString() -> *const c_char;
    pub fn gdFontCacheShutdown();
    pub fn gdFree(p: *mut c_void);
    pub fn gdImageCreate(sx: c_int, sy: c_int) -> Im;
    pub fn gdImageCreateTrueColor(sx: c_int, sy: c_int) -> Im;
    pub fn gdImageDestroy(im: Im);
    pub fn gdNewDynamicCtxEx(size: c_int, data: *mut c_void, free_flag: c_int) -> *mut GdIoCtx;

    pub fn gdImageCreateFromPngCtx(c: *mut GdIoCtx) -> Im;
    pub fn gdImageCreateFromGifCtx(c: *mut GdIoCtx) -> Im;
    pub fn gdImageCreateFromWBMPCtx(c: *mut GdIoCtx) -> Im;
    pub fn gdImageCreateFromJpegCtx(c: *mut GdIoCtx) -> Im;
    pub fn gdImageCreateFromJpegCtxEx(c: *mut GdIoCtx, ignore_warning: c_int) -> Im;
    pub fn gdImageCreateFromWebpCtx(c: *mut GdIoCtx) -> Im;
    pub fn gdImageCreateFromAvifCtx(c: *mut GdIoCtx) -> Im;
    pub fn gdImageCreateFromTgaCtx(c: *mut GdIoCtx) -> Im;
    pub fn gdImageCreateFromBmpCtx(c: *mut GdIoCtx) -> Im;
    pub fn gdImageCreateFromGdCtx(c: *mut GdIoCtx) -> Im;
    pub fn gdImageCreateFromGd2Ctx(c: *mut GdIoCtx) -> Im;
    pub fn gdImageCreateFromGd2PartCtx(
        c: *mut GdIoCtx,
        srcx: c_int,
        srcy: c_int,
        w: c_int,
        h: c_int,
    ) -> Im;

    pub fn gdImageXbmCtx(im: Im, file_name: *mut c_char, fg: c_int, out: *mut GdIoCtx);
    pub fn gdImageGifCtx(im: Im, out: *mut GdIoCtx);
    pub fn gdImageBmpCtx(im: Im, out: *mut GdIoCtx, compression: c_int);
    pub fn gdImagePngCtxEx(im: Im, out: *mut GdIoCtx, level: c_int);
    pub fn gdImageWBMPCtx(im: Im, fg: c_int, out: *mut GdIoCtx);
    pub fn gdImageJpegCtx(im: Im, out: *mut GdIoCtx, quality: c_int);
    pub fn gdImageWebpCtx(im: Im, out: *mut GdIoCtx, quantization: c_int);
    pub fn gdImageAvifCtx(im: Im, out: *mut GdIoCtx, quality: c_int, speed: c_int);
    pub fn gdImageGdPtr(im: Im, size: *mut c_int) -> *mut c_void;
    pub fn gdImageGd2Ptr(im: Im, cs: c_int, fmt: c_int, size: *mut c_int) -> *mut c_void;

    pub fn gdImageSetPixel(im: Im, x: c_int, y: c_int, color: c_int);
    pub fn gdImageGetPixel(im: Im, x: c_int, y: c_int) -> c_int;
    pub fn gdImageLine(im: Im, x1: c_int, y1: c_int, x2: c_int, y2: c_int, color: c_int);
    pub fn gdImageDashedLine(im: Im, x1: c_int, y1: c_int, x2: c_int, y2: c_int, color: c_int);
    pub fn gdImageRectangle(im: Im, x1: c_int, y1: c_int, x2: c_int, y2: c_int, color: c_int);
    pub fn gdImageFilledRectangle(im: Im, x1: c_int, y1: c_int, x2: c_int, y2: c_int, c: c_int);
    pub fn gdImageSetClip(im: Im, x1: c_int, y1: c_int, x2: c_int, y2: c_int);
    pub fn gdImageGetClip(im: Im, x1: *mut c_int, y1: *mut c_int, x2: *mut c_int, y2: *mut c_int);
    pub fn gdImageSetResolution(im: Im, res_x: c_uint, res_y: c_uint);
    pub fn gdImageChar(im: Im, f: *mut GdFont, x: c_int, y: c_int, c: c_int, color: c_int);
    pub fn gdImageCharUp(im: Im, f: *mut GdFont, x: c_int, y: c_int, c: c_int, color: c_int);
    pub fn gdImagePolygon(im: Im, p: *mut GdPoint, n: c_int, c: c_int);
    pub fn gdImageOpenPolygon(im: Im, p: *mut GdPoint, n: c_int, c: c_int);
    pub fn gdImageFilledPolygon(im: Im, p: *mut GdPoint, n: c_int, c: c_int);
    pub fn gdImageColorAllocate(im: Im, r: c_int, g: c_int, b: c_int) -> c_int;
    pub fn gdImageColorAllocateAlpha(im: Im, r: c_int, g: c_int, b: c_int, a: c_int) -> c_int;
    pub fn gdImageColorClosest(im: Im, r: c_int, g: c_int, b: c_int) -> c_int;
    pub fn gdImageColorClosestAlpha(im: Im, r: c_int, g: c_int, b: c_int, a: c_int) -> c_int;
    pub fn gdImageColorClosestHWB(im: Im, r: c_int, g: c_int, b: c_int) -> c_int;
    pub fn gdImageColorExact(im: Im, r: c_int, g: c_int, b: c_int) -> c_int;
    pub fn gdImageColorExactAlpha(im: Im, r: c_int, g: c_int, b: c_int, a: c_int) -> c_int;
    pub fn gdImageColorResolve(im: Im, r: c_int, g: c_int, b: c_int) -> c_int;
    pub fn gdImageColorResolveAlpha(im: Im, r: c_int, g: c_int, b: c_int, a: c_int) -> c_int;
    pub fn gdImageColorDeallocate(im: Im, color: c_int);
    pub fn gdImageTrueColorToPalette(im: Im, dither: c_int, colors_wanted: c_int) -> c_int;
    pub fn gdImagePaletteToTrueColor(src: Im) -> c_int;
    pub fn gdImageColorMatch(im1: Im, im2: Im) -> c_int;
    pub fn gdImageColorTransparent(im: Im, color: c_int);
    pub fn gdImagePaletteCopy(dst: Im, src: Im);
    pub fn gdImageFilledArc(
        im: Im,
        cx: c_int,
        cy: c_int,
        w: c_int,
        h: c_int,
        s: c_int,
        e: c_int,
        color: c_int,
        style: c_int,
    );
    pub fn gdImageArc(
        im: Im,
        cx: c_int,
        cy: c_int,
        w: c_int,
        h: c_int,
        s: c_int,
        e: c_int,
        color: c_int,
    );
    pub fn gdImageEllipse(im: Im, cx: c_int, cy: c_int, w: c_int, h: c_int, color: c_int);
    pub fn gdImageFilledEllipse(im: Im, cx: c_int, cy: c_int, w: c_int, h: c_int, color: c_int);
    pub fn gdImageFillToBorder(im: Im, x: c_int, y: c_int, border: c_int, color: c_int);
    pub fn gdImageFill(im: Im, x: c_int, y: c_int, color: c_int);
    pub fn gdImageCopy(
        dst: Im,
        src: Im,
        dx: c_int,
        dy: c_int,
        sx: c_int,
        sy: c_int,
        w: c_int,
        h: c_int,
    );
    pub fn gdImageCopyMerge(
        dst: Im,
        src: Im,
        dx: c_int,
        dy: c_int,
        sx: c_int,
        sy: c_int,
        w: c_int,
        h: c_int,
        pct: c_int,
    );
    pub fn gdImageCopyMergeGray(
        dst: Im,
        src: Im,
        dx: c_int,
        dy: c_int,
        sx: c_int,
        sy: c_int,
        w: c_int,
        h: c_int,
        pct: c_int,
    );
    pub fn gdImageCopyResized(
        dst: Im,
        src: Im,
        dx: c_int,
        dy: c_int,
        sx: c_int,
        sy: c_int,
        dw: c_int,
        dh: c_int,
        sw: c_int,
        sh: c_int,
    );
    pub fn gdImageCopyResampled(
        dst: Im,
        src: Im,
        dx: c_int,
        dy: c_int,
        sx: c_int,
        sy: c_int,
        dw: c_int,
        dh: c_int,
        sw: c_int,
        sh: c_int,
    );
    pub fn gdImageSetBrush(im: Im, brush: Im);
    pub fn gdImageSetTile(im: Im, tile: Im);
    pub fn gdImageSetAntiAliased(im: Im, c: c_int);
    pub fn gdImageSetStyle(im: Im, style: *mut c_int, n: c_int);
    pub fn gdImageSetThickness(im: Im, thickness: c_int);
    pub fn gdImageInterlace(im: Im, interlace: c_int);
    pub fn gdImageAlphaBlending(im: Im, blending: c_int);
    pub fn gdImageSaveAlpha(im: Im, save: c_int);
    pub fn gdImagePixelate(im: Im, block_size: c_int, mode: c_uint) -> c_int;
    pub fn gdImageScatter(im: Im, sub: c_int, plus: c_int) -> c_int;
    pub fn gdImageScatterColor(
        im: Im,
        sub: c_int,
        plus: c_int,
        colors: *mut c_int,
        num: c_uint,
    ) -> c_int;
    pub fn gdImageSmooth(im: Im, weight: c_float) -> c_int;
    pub fn gdImageMeanRemoval(im: Im) -> c_int;
    pub fn gdImageEmboss(im: Im) -> c_int;
    pub fn gdImageGaussianBlur(im: Im) -> c_int;
    pub fn gdImageEdgeDetectQuick(im: Im) -> c_int;
    pub fn gdImageSelectiveBlur(im: Im) -> c_int;
    pub fn gdImageConvolution(
        im: Im,
        filter: *mut [c_float; 3],
        div: c_float,
        offset: c_float,
    ) -> c_int;
    pub fn gdImageColor(im: Im, r: c_int, g: c_int, b: c_int, a: c_int) -> c_int;
    pub fn gdImageContrast(im: Im, contrast: c_double) -> c_int;
    pub fn gdImageBrightness(im: Im, brightness: c_int) -> c_int;
    pub fn gdImageGrayScale(im: Im) -> c_int;
    pub fn gdImageNegate(im: Im) -> c_int;
    pub fn gdImageFlipHorizontal(im: Im);
    pub fn gdImageFlipVertical(im: Im);
    pub fn gdImageFlipBoth(im: Im);
    pub fn gdImageCrop(src: Im, crop: *const GdRect) -> Im;
    pub fn gdImageCropAuto(im: Im, mode: c_uint) -> Im;
    pub fn gdImageCropThreshold(im: Im, color: c_uint, threshold: c_float) -> Im;
    pub fn gdImageSetInterpolationMethod(im: Im, id: c_int) -> c_int;
    pub fn gdImageGetInterpolationMethod(im: Im) -> c_int;
    pub fn gdImageScale(src: Im, w: c_uint, h: c_uint) -> Im;
    pub fn gdImageRotateInterpolated(src: Im, angle: c_float, bg: c_int) -> Im;
    pub fn gdAffineConcat(dst: *mut c_double, m1: *const c_double, m2: *const c_double) -> c_int;
    pub fn gdAffineScale(dst: *mut c_double, x: c_double, y: c_double) -> c_int;
    pub fn gdAffineRotate(dst: *mut c_double, angle: c_double) -> c_int;
    pub fn gdAffineShearHorizontal(dst: *mut c_double, angle: c_double) -> c_int;
    pub fn gdAffineShearVertical(dst: *mut c_double, angle: c_double) -> c_int;
    pub fn gdAffineTranslate(dst: *mut c_double, x: c_double, y: c_double) -> c_int;
    pub fn gdTransformAffineGetImage(
        dst: *mut Im,
        src: Im,
        area: *mut GdRect,
        affine: *const c_double,
    ) -> c_int;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn layouts_match_gd_h() {
        let want = [
            size_of::<GdIoCtx>(),
            offset_of!(GdIoCtx, put_c),
            offset_of!(GdIoCtx, put_buf),
            offset_of!(GdIoCtx, gd_free),
            offset_of!(GdIoCtx, data),
            size_of::<GdFont>(),
            offset_of!(GdFont, data),
            size_of::<GdRect>(),
            size_of::<GdPoint>(),
        ];
        for (i, w) in want.iter().enumerate() {
            assert_eq!(unsafe { rhp_gd_layout(i as c_int) }, *w, "layout {i}");
        }
    }

    /// libgd is linked dynamically from the directory the build found it
    /// in, and is the release of the gd.h rhp compiled against.
    #[test]
    fn libgd_is_the_library_php_links() {
        let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
        let ok = unsafe { libc::dladdr(gdImageCreate as *const c_void, &mut info) };
        assert!(
            ok != 0 && !info.dli_fname.is_null(),
            "dladdr finds gdImageCreate"
        );
        let path = unsafe { std::ffi::CStr::from_ptr(info.dli_fname) }
            .to_string_lossy()
            .into_owned();
        let canon = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.into());
        if let Some(libdir) = option_env!("RHP_LIBGD_LIBDIR") {
            let dir = canon(std::path::Path::new(&path))
                .parent()
                .map(std::path::Path::to_path_buf);
            assert_eq!(
                dir,
                Some(canon(std::path::Path::new(libdir))),
                "gdImageCreate comes from {path}"
            );
        }
        let lib = unsafe { std::ffi::CStr::from_ptr(gdVersionString()) };
        let hdr = unsafe { std::ffi::CStr::from_ptr(rhp_gd_version_string()) };
        assert_eq!(lib, hdr, "the library is the release of the headers");
    }
}
