/* ext/gd's C side: what gd.c reads through gd.h's macros and struct
 * fields, the constants of gd.stub.php evaluated against the linked gd.h,
 * and the error callback php installs (php_gd_error_method). */

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <gd.h>
#include <gd_errors.h>
#include <gdfontt.h>
#include <gdfonts.h>
#include <gdfontmb.h>
#include <gdfontl.h>
#include <gdfontg.h>

/* libgd is opened on first use (sys.rs): its functions and data come
 * from rhp_gd_sym, looked up once per call site. */
extern void *rhp_gd_sym(const char *name);
#define GD(name) (*({ \
	static __typeof__(&name) gd_p_; \
	__typeof__(&name) gd_f_ = __atomic_load_n(&gd_p_, __ATOMIC_ACQUIRE); \
	if (!gd_f_) { \
		gd_f_ = (__typeof__(&name)) rhp_gd_sym(#name); \
		__atomic_store_n(&gd_p_, gd_f_, __ATOMIC_RELEASE); \
	} \
	gd_f_; \
}))

/* ---- errors ---- */

/* Defined in Rust (ext/gd/mod.rs). */
extern void rhp_gd_error(int type, const char *msg, size_t len);

/* php_gd_error_method: libgd's message formatted, then raised from the
 * running builtin. */
void rhp_gd_error_method(int type, const char *format, va_list args)
{
	char small[512];
	va_list copy;
	va_copy(copy, args);
	int n = vsnprintf(small, sizeof small, format, copy);
	va_end(copy);
	if (n < 0) {
		return;
	}
	if ((size_t) n < sizeof small) {
		rhp_gd_error(type, small, (size_t) n);
		return;
	}
	char *big = malloc((size_t) n + 1);
	if (!big) {
		return;
	}
	vsnprintf(big, (size_t) n + 1, format, args);
	rhp_gd_error(type, big, (size_t) n);
	free(big);
}

/* php's HAVE_GD_* from configure: build.rs runs the same probe. */
int rhp_gd_features(void)
{
	return RHP_GD_FORMATS;
}

gdImagePtr rhp_gd_from_xpm(const char *filename)
{
	return GD(gdImageCreateFromXpm)((char *) filename);
}

int rhp_gd_level(int which)
{
	switch (which) {
		case 0: return GD_ERROR;
		case 1: return GD_WARNING;
		case 2: return GD_NOTICE;
		case 3: return GD_INFO;
		default: return GD_DEBUG;
	}
}

/* ---- the image struct ---- */

int rhp_gd_sx(gdImagePtr im) { return gdImageSX(im); }
int rhp_gd_sy(gdImagePtr im) { return gdImageSY(im); }
int rhp_gd_truecolor(gdImagePtr im) { return gdImageTrueColor(im); }
int rhp_gd_colors_total(gdImagePtr im) { return gdImageColorsTotal(im); }
int rhp_gd_red(gdImagePtr im, int c) { return gdImageRed(im, c); }
int rhp_gd_green(gdImagePtr im, int c) { return gdImageGreen(im, c); }
int rhp_gd_blue(gdImagePtr im, int c) { return gdImageBlue(im, c); }
int rhp_gd_alpha(gdImagePtr im, int c) { return gdImageAlpha(im, c); }
int rhp_gd_get_transparent(gdImagePtr im) { return gdImageGetTransparent(im); }
int rhp_gd_get_interlaced(gdImagePtr im) { return gdImageGetInterlaced(im); }
int rhp_gd_aa(gdImagePtr im) { return im->AA; }
void rhp_gd_set_aa(gdImagePtr im, int aa) { im->AA = aa; }
int rhp_gd_interpolation_id(gdImagePtr im) { return (int) im->interpolation_id; }
unsigned int rhp_gd_res_x(gdImagePtr im) { return gdImageResolutionX(im); }
unsigned int rhp_gd_res_y(gdImagePtr im) { return gdImageResolutionY(im); }

/* imagecolorat: 1 and the pixel, or 0 when (x, y) is out of bounds. */
int rhp_gd_color_at(gdImagePtr im, long x, long y, long *out)
{
	if (gdImageTrueColor(im)) {
		if (im->tpixels && GD(gdImageBoundsSafe)(im, x, y)) {
			*out = gdImageTrueColorPixel(im, x, y);
			return 1;
		}
	} else if (im->pixels && GD(gdImageBoundsSafe)(im, x, y)) {
		*out = im->pixels[y][x];
		return 1;
	}
	return 0;
}

/* imagecolorset's palette write. */
void rhp_gd_set_palette(gdImagePtr im, int c, int r, int g, int b, int a)
{
	im->red[c] = r;
	im->green[c] = g;
	im->blue[c] = b;
	im->alpha[c] = a;
}

/* imagegammacorrect's palette write (alpha kept). */
void rhp_gd_set_rgb(gdImagePtr im, int c, int r, int g, int b)
{
	im->red[c] = r;
	im->green[c] = g;
	im->blue[c] = b;
}

/* ---- fonts ---- */

gdFontPtr rhp_gd_builtin_font(int n)
{
	switch (n) {
		case 1: return GD(gdFontTiny);
		case 2: return GD(gdFontSmall);
		case 3: return GD(gdFontMediumBold);
		case 4: return GD(gdFontLarge);
		default: return GD(gdFontGiant);
	}
}

/* imagefttext with an options array (gdImageStringFTEx) or without. */
char *rhp_gd_string_ft(gdImagePtr im, int *brect, int fg, const char *font,
	double ptsize, double angle, int x, int y, const char *str,
	int ext, int linespace, double linespacing)
{
	if (ext) {
		gdFTStringExtra strex;
		memset(&strex, 0, sizeof strex);
		if (linespace) {
			strex.flags |= gdFTEX_LINESPACE;
			strex.linespacing = linespacing;
		}
		return GD(gdImageStringFTEx)(im, brect, fg, (char *) font, ptsize, angle, x, y, (char *) str, &strex);
	}
	return GD(gdImageStringFT)(im, brect, fg, (char *) font, ptsize, angle, x, y, (char *) str);
}

/* ---- reading through a FILE* (imagecreatefromxbm) ---- */

gdImagePtr rhp_gd_from_xbm(const void *data, size_t len)
{
	FILE *fp = fmemopen((void *) data, len, "rb");
	if (!fp) {
		return NULL;
	}
	gdImagePtr im = GD(gdImageCreateFromXbm)(fp);
	fclose(fp);
	return im;
}

/* ---- constants (gd.stub.php's @cvalue) ---- */

struct rhp_gd_const {
	const char *name;
	long value;
};

static const struct rhp_gd_const consts[] = {
#ifdef gdWebpLossless
	{"IMG_WEBP_LOSSLESS", gdWebpLossless},
#endif
	{"IMG_COLOR_TILED", gdTiled},
	{"IMG_COLOR_STYLED", gdStyled},
	{"IMG_COLOR_BRUSHED", gdBrushed},
	{"IMG_COLOR_STYLEDBRUSHED", gdStyledBrushed},
	{"IMG_COLOR_TRANSPARENT", gdTransparent},
	{"IMG_ARC_ROUNDED", gdArc},
	{"IMG_ARC_PIE", gdPie},
	{"IMG_ARC_CHORD", gdChord},
	{"IMG_ARC_NOFILL", gdNoFill},
	{"IMG_ARC_EDGED", gdEdged},
	{"IMG_GD2_RAW", GD2_FMT_RAW},
	{"IMG_GD2_COMPRESSED", GD2_FMT_COMPRESSED},
	{"IMG_EFFECT_REPLACE", gdEffectReplace},
	{"IMG_EFFECT_ALPHABLEND", gdEffectAlphaBlend},
	{"IMG_EFFECT_NORMAL", gdEffectNormal},
	{"IMG_EFFECT_OVERLAY", gdEffectOverlay},
#ifdef gdEffectMultiply
	{"IMG_EFFECT_MULTIPLY", gdEffectMultiply},
#endif
	{"IMG_CROP_DEFAULT", GD_CROP_DEFAULT},
	{"IMG_CROP_TRANSPARENT", GD_CROP_TRANSPARENT},
	{"IMG_CROP_BLACK", GD_CROP_BLACK},
	{"IMG_CROP_WHITE", GD_CROP_WHITE},
	{"IMG_CROP_SIDES", GD_CROP_SIDES},
	{"IMG_CROP_THRESHOLD", GD_CROP_THRESHOLD},
	{"IMG_BELL", GD_BELL},
	{"IMG_BESSEL", GD_BESSEL},
	{"IMG_BILINEAR_FIXED", GD_BILINEAR_FIXED},
	{"IMG_BICUBIC", GD_BICUBIC},
	{"IMG_BICUBIC_FIXED", GD_BICUBIC_FIXED},
	{"IMG_BLACKMAN", GD_BLACKMAN},
	{"IMG_BOX", GD_BOX},
	{"IMG_BSPLINE", GD_BSPLINE},
	{"IMG_CATMULLROM", GD_CATMULLROM},
	{"IMG_GAUSSIAN", GD_GAUSSIAN},
	{"IMG_GENERALIZED_CUBIC", GD_GENERALIZED_CUBIC},
	{"IMG_HERMITE", GD_HERMITE},
	{"IMG_HAMMING", GD_HAMMING},
	{"IMG_HANNING", GD_HANNING},
	{"IMG_MITCHELL", GD_MITCHELL},
	{"IMG_POWER", GD_POWER},
	{"IMG_QUADRATIC", GD_QUADRATIC},
	{"IMG_SINC", GD_SINC},
	{"IMG_NEAREST_NEIGHBOUR", GD_NEAREST_NEIGHBOUR},
	{"IMG_WEIGHTED4", GD_WEIGHTED4},
	{"IMG_TRIANGLE", GD_TRIANGLE},
	{"IMG_AFFINE_TRANSLATE", GD_AFFINE_TRANSLATE},
	{"IMG_AFFINE_SCALE", GD_AFFINE_SCALE},
	{"IMG_AFFINE_ROTATE", GD_AFFINE_ROTATE},
	{"IMG_AFFINE_SHEAR_HORIZONTAL", GD_AFFINE_SHEAR_HORIZONTAL},
	{"IMG_AFFINE_SHEAR_VERTICAL", GD_AFFINE_SHEAR_VERTICAL},
	{"GD_MAJOR_VERSION", GD_MAJOR_VERSION},
	{"GD_MINOR_VERSION", GD_MINOR_VERSION},
	{"GD_RELEASE_VERSION", GD_RELEASE_VERSION},
	/* Not stub constants: what gd.c compares against. */
	{"=GD_METHOD_COUNT", GD_METHOD_COUNT},
	{"=GD_DEFAULT", GD_DEFAULT},
	{NULL, 0},
};

const struct rhp_gd_const *rhp_gd_consts(void)
{
	return consts;
}

const char *rhp_gd_version_string(void) { return GD_VERSION_STRING; }
const char *rhp_gd_extra_version(void) { return GD_EXTRA_VERSION; }

/* The layouts sys.rs mirrors (its tests compare). */
#include <stddef.h>
size_t rhp_gd_layout(int which)
{
	switch (which) {
		case 0: return sizeof(gdIOCtx);
		case 1: return offsetof(gdIOCtx, putC);
		case 2: return offsetof(gdIOCtx, putBuf);
		case 3: return offsetof(gdIOCtx, gd_free);
		case 4: return offsetof(gdIOCtx, data);
		case 5: return sizeof(gdFont);
		case 6: return offsetof(gdFont, data);
		case 7: return sizeof(gdRect);
		case 8: return sizeof(gdPoint);
		default: return 0;
	}
}
