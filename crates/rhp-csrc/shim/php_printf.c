/*
 * php's own printf, which php.h puts in place of the C library's for all
 * php-src C: a port of main/snprintf.c's format_converter and
 * main/spprintf.c's xbuf_format_converter (PHP License 3.01, with parts
 * under the Apache Group's licence; see THIRD-PARTY-NOTICES). One converter
 * serves both: `sp` selects spprintf's few differences (%S, "nan"/"inf"
 * for %e and %f, strnlen for a precision on %s). The digits of %e, %f and
 * %g come from rhp's ports of php_conv_fp and zend_gcvt (src/lib.rs).
 */
#include <ctype.h>
#include <limits.h>
#include <locale.h>
#include <math.h>
#include <stdio.h>
#include <string.h>

#include "zend_shim.h"

/* src/lib.rs */
size_t rhp_csrc_conv_fp(char format, double num, bool add_dp, int precision, char dec_point,
	bool *is_negative, char *buf, size_t cap);
size_t rhp_csrc_gcvt(double value, int ndigit, char dec_point, char exp_char, char *buf, size_t cap);

#define NUL '\0'
#define S_NULL "(null)"
#define S_NULL_LEN 6
#define FLOAT_DIGITS 6
/* main/snprintf.c's NUM_BUF_SIZE, the larger of the two */
#define NUM_BUF_SIZE 2048

typedef enum {
	LM_STD = 0,
	LM_INTMAX_T,
	LM_PTRDIFF_T,
	LM_LONG_LONG,
	LM_SIZE_T,
	LM_LONG,
	LM_LONG_DOUBLE,
} length_modifier_e;

/* A growing output buffer (smart_string). */
typedef struct {
	char *c;
	size_t len;
	size_t a;
} xbuf;

static void xbuf_reserve(xbuf *b, size_t n)
{
	if (b->len + n + 1 > b->a) {
		size_t a = b->a ? b->a : 128;
		while (a < b->len + n + 1) {
			a *= 2;
		}
		b->c = erealloc(b->c, a);
		b->a = a;
	}
}

static void xbuf_appendl(xbuf *b, const char *s, size_t n)
{
	xbuf_reserve(b, n);
	memcpy(b->c + b->len, s, n);
	b->len += n;
}

static void xbuf_pad(xbuf *b, char ch, size_t n)
{
	xbuf_reserve(b, n);
	memset(b->c + b->len, ch, n);
	b->len += n;
}

/* ap_php_conv_10 */
static char *conv_10(int64_t num, bool is_unsigned, bool *is_negative, char *buf_end, size_t *len)
{
	char *p = buf_end;
	uint64_t magnitude;

	if (is_unsigned) {
		magnitude = (uint64_t)num;
		*is_negative = false;
	} else {
		*is_negative = (num < 0);
		if (*is_negative) {
			int64_t t = num + 1;
			magnitude = ((uint64_t)-t) + 1;
		} else {
			magnitude = (uint64_t)num;
		}
	}
	do {
		uint64_t new_magnitude = magnitude / 10;
		*--p = (char)(magnitude - new_magnitude * 10 + '0');
		magnitude = new_magnitude;
	} while (magnitude);
	*len = buf_end - p;
	return p;
}

/* ap_php_conv_p2 */
static char *conv_p2(uint64_t num, int nbits, char format, char *buf_end, size_t *len)
{
	int mask = (1 << nbits) - 1;
	char *p = buf_end;
	static const char low_digits[] = "0123456789abcdef";
	static const char upper_digits[] = "0123456789ABCDEF";
	const char *digits = (format == 'X') ? upper_digits : low_digits;

	do {
		*--p = digits[num & mask];
		num >>= nbits;
	} while (num);
	*len = buf_end - p;
	return p;
}

#define NUM(c) (c - '0')

/* spprintf's STR_TO_DEC stops growing at INT_MAX / 10; snprintf's does
 * not, and overflows as php's does (wrapping: -fwrapv). */
#define STR_TO_DEC(str, num) do { \
	num = NUM(*(str)++); \
	while (isdigit((unsigned char)*(str))) { \
		num *= 10; \
		num += NUM(*(str)++); \
		if (sp && num >= INT_MAX / 10) { \
			while (isdigit((unsigned char)*(str)++)); \
			break; \
		} \
	} \
} while (0)

#define FIX_PRECISION(adjust, precision, s, s_len) do { \
	if (adjust) \
		while (s_len < (size_t)precision) { \
			*--s = '0'; \
			s_len++; \
		} \
} while (0)

static void format_converter(xbuf *out, bool sp, const char *fmt, va_list ap)
{
	char *s = NULL;
	size_t s_len;
	int min_width = 0;
	int precision = 0;
	enum { LEFT, RIGHT } adjust;
	char pad_char;
	char prefix_char;
	double fp_num;
	int64_t i_num = 0;
	uint64_t ui_num = 0;
	char num_buf[NUM_BUF_SIZE];
	char char_buf[2];
	struct lconv *lconv = NULL;
	length_modifier_e modifier;
	bool alternate_form, print_sign, print_blank, adjust_precision, adjust_width, is_negative;

	while (*fmt) {
		if (*fmt != '%') {
			xbuf_appendl(out, fmt, 1);
		} else {
			adjust = RIGHT;
			alternate_form = print_sign = print_blank = false;
			pad_char = ' ';
			prefix_char = NUL;

			fmt++;

			if (isascii((unsigned char)*fmt) && !islower((unsigned char)*fmt)) {
				for (;; fmt++) {
					if (*fmt == '-')
						adjust = LEFT;
					else if (*fmt == '+')
						print_sign = true;
					else if (*fmt == '#')
						alternate_form = true;
					else if (*fmt == ' ')
						print_blank = true;
					else if (*fmt == '0')
						pad_char = '0';
					else
						break;
				}
				if (isdigit((unsigned char)*fmt)) {
					STR_TO_DEC(fmt, min_width);
					adjust_width = true;
				} else if (*fmt == '*') {
					min_width = va_arg(ap, int);
					fmt++;
					adjust_width = true;
					if (min_width < 0) {
						adjust = LEFT;
						min_width = -min_width;
					}
				} else {
					adjust_width = false;
				}
				if (*fmt == '.') {
					adjust_precision = true;
					fmt++;
					if (isdigit((unsigned char)*fmt)) {
						STR_TO_DEC(fmt, precision);
					} else if (*fmt == '*') {
						precision = va_arg(ap, int);
						fmt++;
						if (precision < -1)
							precision = -1;
					} else {
						precision = 0;
					}
				} else {
					adjust_precision = false;
				}
			} else {
				adjust_precision = adjust_width = false;
			}

			switch (*fmt) {
				case 'L':
					fmt++;
					modifier = LM_LONG_DOUBLE;
					break;
				case 'l':
					fmt++;
					if (*fmt == 'l') {
						fmt++;
						modifier = LM_LONG_LONG;
					} else {
						modifier = LM_LONG;
					}
					break;
				case 'z':
					fmt++;
					modifier = LM_SIZE_T;
					break;
				case 'j':
					fmt++;
					modifier = LM_INTMAX_T;
					break;
				case 't':
					fmt++;
					modifier = LM_PTRDIFF_T;
					break;
				case 'p': {
					char next = *(fmt + 1);
					if ('d' == next || 'u' == next || 'x' == next || 'o' == next) {
						zend_error_noreturn(E_CORE_ERROR,
							"printf \"p\" modifier is no longer supported, use ZEND_LONG_FMT");
					}
					modifier = LM_STD;
					break;
				}
				case 'h':
					fmt++;
					if (*fmt == 'h') {
						fmt++;
					}
					ZEND_FALLTHROUGH;
				default:
					modifier = LM_STD;
					break;
			}

			switch (*fmt) {
				case 'S':
					if (!sp) {
						goto unknown;
					}
					{
						zend_string *str = va_arg(ap, zend_string *);
						s_len = ZSTR_LEN(str);
						s = ZSTR_VAL(str);
						if (adjust_precision && (size_t)precision < s_len) {
							s_len = precision;
						}
					}
					break;
				case 'u':
					switch (modifier) {
						default:
							i_num = (int64_t)va_arg(ap, unsigned int);
							break;
						case LM_LONG_DOUBLE:
							goto fmt_error;
						case LM_LONG:
							i_num = (int64_t)va_arg(ap, unsigned long int);
							break;
						case LM_SIZE_T:
							i_num = (int64_t)va_arg(ap, size_t);
							break;
						case LM_LONG_LONG:
							i_num = (int64_t)va_arg(ap, unsigned long long int);
							break;
						case LM_INTMAX_T:
							i_num = (int64_t)va_arg(ap, uintmax_t);
							break;
						case LM_PTRDIFF_T:
							i_num = (int64_t)va_arg(ap, ptrdiff_t);
							break;
					}
					ZEND_FALLTHROUGH;
				case 'd':
				case 'i':
					if ((*fmt) != 'u') {
						switch (modifier) {
							default:
								i_num = (int64_t)va_arg(ap, int);
								break;
							case LM_LONG_DOUBLE:
								goto fmt_error;
							case LM_LONG:
								i_num = (int64_t)va_arg(ap, long int);
								break;
							case LM_SIZE_T:
								i_num = (int64_t)va_arg(ap, ssize_t);
								break;
							case LM_LONG_LONG:
								i_num = (int64_t)va_arg(ap, long long int);
								break;
							case LM_INTMAX_T:
								i_num = (int64_t)va_arg(ap, intmax_t);
								break;
							case LM_PTRDIFF_T:
								i_num = (int64_t)va_arg(ap, ptrdiff_t);
								break;
						}
					}
					s = conv_10(i_num, (*fmt) == 'u', &is_negative, &num_buf[NUM_BUF_SIZE], &s_len);
					FIX_PRECISION(adjust_precision, precision, s, s_len);
					if (*fmt != 'u') {
						if (is_negative)
							prefix_char = '-';
						else if (print_sign)
							prefix_char = '+';
						else if (print_blank)
							prefix_char = ' ';
					}
					break;

				case 'o':
				case 'x':
				case 'X':
					switch (modifier) {
						default:
							ui_num = (uint64_t)va_arg(ap, unsigned int);
							break;
						case LM_LONG_DOUBLE:
							goto fmt_error;
						case LM_LONG:
							ui_num = (uint64_t)va_arg(ap, unsigned long int);
							break;
						case LM_SIZE_T:
							ui_num = (uint64_t)va_arg(ap, size_t);
							break;
						case LM_LONG_LONG:
							ui_num = (uint64_t)va_arg(ap, unsigned long long int);
							break;
						case LM_INTMAX_T:
							ui_num = (uint64_t)va_arg(ap, uintmax_t);
							break;
						case LM_PTRDIFF_T:
							ui_num = (uint64_t)va_arg(ap, ptrdiff_t);
							break;
					}
					if (*fmt == 'o') {
						s = conv_p2(ui_num, 3, *fmt, &num_buf[NUM_BUF_SIZE], &s_len);
						FIX_PRECISION(adjust_precision, precision, s, s_len);
						if (alternate_form && *s != '0') {
							*--s = '0';
							s_len++;
						}
					} else {
						s = conv_p2(ui_num, 4, *fmt, &num_buf[NUM_BUF_SIZE], &s_len);
						FIX_PRECISION(adjust_precision, precision, s, s_len);
						if (alternate_form && ui_num != 0) {
							*--s = *fmt;
							*--s = '0';
							s_len += 2;
						}
					}
					break;

				case 's':
					s = va_arg(ap, char *);
					if (s != NULL) {
						if (sp && adjust_precision) {
							s_len = strnlen(s, precision);
						} else {
							s_len = strlen(s);
							if (adjust_precision && (size_t)precision < s_len) {
								s_len = precision;
							}
						}
					} else {
						s = S_NULL;
						s_len = S_NULL_LEN;
					}
					pad_char = ' ';
					break;

				case 'f':
				case 'F':
				case 'e':
				case 'E':
					switch (modifier) {
						case LM_LONG_DOUBLE:
							fp_num = (double)va_arg(ap, long double);
							break;
						case LM_STD:
							fp_num = va_arg(ap, double);
							break;
						default:
							goto fmt_error;
					}
					if (isnan(fp_num)) {
						s = sp ? "nan" : "NAN";
						s_len = 3;
					} else if (isinf(fp_num)) {
						s = sp ? "inf" : "INF";
						s_len = 3;
					} else {
						if (!lconv) {
							lconv = localeconv();
						}
						s = &num_buf[1];
						s_len = rhp_csrc_conv_fp((*fmt == 'f') ? 'F' : *fmt, fp_num, alternate_form,
							(adjust_precision == false) ? FLOAT_DIGITS : precision,
							(*fmt == 'f') ? *lconv->decimal_point : '.',
							&is_negative, &num_buf[1], NUM_BUF_SIZE - 1);
						if (is_negative)
							prefix_char = '-';
						else if (print_sign)
							prefix_char = '+';
						else if (print_blank)
							prefix_char = ' ';
					}
					break;

				case 'g':
				case 'k':
				case 'G':
				case 'H':
					switch (modifier) {
						case LM_LONG_DOUBLE:
							fp_num = (double)va_arg(ap, long double);
							break;
						case LM_STD:
							fp_num = va_arg(ap, double);
							break;
						default:
							goto fmt_error;
					}
					if (isnan(fp_num)) {
						s = "NAN";
						s_len = 3;
						break;
					} else if (isinf(fp_num)) {
						if (fp_num > 0) {
							s = "INF";
							s_len = 3;
						} else {
							s = "-INF";
							s_len = 4;
						}
						break;
					}
					if (adjust_precision == false)
						precision = FLOAT_DIGITS;
					else if (precision == 0)
						precision = 1;
					if (!lconv) {
						lconv = localeconv();
					}
					/* &num_buf[1], so that there is room for the sign */
					s = &num_buf[1];
					rhp_csrc_gcvt(fp_num, precision,
						(*fmt == 'H' || *fmt == 'k') ? '.' : *lconv->decimal_point,
						(*fmt == 'G' || *fmt == 'H') ? 'E' : 'e', &num_buf[1], NUM_BUF_SIZE - 2);
					if (*s == '-')
						prefix_char = *s++;
					else if (print_sign)
						prefix_char = '+';
					else if (print_blank)
						prefix_char = ' ';
					s_len = strlen(s);
					if (alternate_form && (strchr(s, '.')) == NULL)
						s[s_len++] = '.';
					break;

				case 'c':
					char_buf[0] = (char)(va_arg(ap, int));
					s = &char_buf[0];
					s_len = 1;
					pad_char = ' ';
					break;

				case '%':
					char_buf[0] = '%';
					s = &char_buf[0];
					s_len = 1;
					pad_char = ' ';
					break;

				case 'n':
					*(va_arg(ap, int *)) = (int)out->len;
					goto skip_output;

				case 'p':
					ui_num = (uint64_t)((size_t)va_arg(ap, char *));
					s = conv_p2(ui_num, 4, 'x', &num_buf[NUM_BUF_SIZE], &s_len);
					if (ui_num != 0) {
						*--s = 'x';
						*--s = '0';
						s_len += 2;
					}
					pad_char = ' ';
					break;

				case NUL:
					/* A trailing %: ignored. */
					continue;

fmt_error:
					php_error(E_ERROR, "Illegal length modifier specified '%c' in s[np]printf call", *fmt);
					ZEND_FALLTHROUGH;
				default:
unknown:
					char_buf[0] = '%';
					char_buf[1] = *fmt;
					s = char_buf;
					s_len = 2;
					pad_char = ' ';
					break;
			}

			if (prefix_char != NUL) {
				*--s = prefix_char;
				s_len++;
			}
			if (adjust_width && adjust == RIGHT && (size_t)min_width > s_len) {
				if (pad_char == '0' && prefix_char != NUL) {
					xbuf_appendl(out, s, 1);
					s++;
					s_len--;
					min_width--;
				}
				xbuf_pad(out, pad_char, min_width - s_len);
			}
			xbuf_appendl(out, s, s_len);
			if (adjust_width && adjust == LEFT && (size_t)min_width > s_len) {
				xbuf_pad(out, pad_char, min_width - s_len);
			}
		}
skip_output:
		fmt++;
	}
}

/* strx_printv: the length of the whole output; at most len - 1 bytes of it
 * and a NUL go to buf. */
static size_t strx_printv(char *buf, size_t len, const char *format, va_list ap)
{
	xbuf out = {NULL, 0, 0};
	format_converter(&out, false, format, ap);
	if (len != 0) {
		size_t n = out.len < len - 1 ? out.len : len - 1;
		if (n) {
			memcpy(buf, out.c, n);
		}
		buf[n] = '\0';
	}
	efree(out.c);
	return out.len;
}

int rhp_csrc_vslprintf(char *buf, size_t len, const char *format, va_list ap)
{
	size_t cc = strx_printv(buf, len, format, ap);
	if (cc >= len) {
		cc = len - 1;
		buf[cc] = '\0';
	}
	return (int)cc;
}

int rhp_csrc_slprintf(char *buf, size_t len, const char *format, ...)
{
	va_list ap;
	va_start(ap, format);
	int cc = rhp_csrc_vslprintf(buf, len, format, ap);
	va_end(ap);
	return cc;
}

int rhp_csrc_vsnprintf(char *buf, size_t len, const char *format, va_list ap)
{
	return (int)strx_printv(buf, len, format, ap);
}

int rhp_csrc_snprintf(char *buf, size_t len, const char *format, ...)
{
	va_list ap;
	va_start(ap, format);
	int cc = (int)strx_printv(buf, len, format, ap);
	va_end(ap);
	return cc;
}

/* zend_vspprintf: an emalloc'd string, cut to max_len bytes when max_len
 * is not 0. */
size_t rhp_csrc_vspprintf(char **pbuf, size_t max_len, const char *format, va_list ap)
{
	xbuf out = {NULL, 0, 0};
	format_converter(&out, true, format, ap);
	if (max_len && out.len > max_len) {
		out.len = max_len;
	}
	if (out.c == NULL) {
		*pbuf = estrndup("", 0);
		return 0;
	}
	out.c[out.len] = '\0';
	*pbuf = out.c;
	return out.len;
}

size_t rhp_csrc_spprintf(char **pbuf, size_t max_len, const char *format, ...)
{
	va_list ap;
	va_start(ap, format);
	size_t cc = rhp_csrc_vspprintf(pbuf, max_len, format, ap);
	va_end(ap);
	return cc;
}
