/*
 * What mbregex.rs needs from oniguruma.h that Rust cannot spell: the
 * encoding and syntax objects (data symbols behind macros), OnigRegion's
 * fields, the match parameters and the variadic onig_error_code_to_str.
 */
#include <stddef.h>
#include <oniguruma.h>

/* In the order of mbregex.rs's ENC_NAME_MAP (php_mbregex.c's enc_name_map
 * without KOI8: php's configure finds that libonig has no OnigEncodingKOI8
 * and leaves it out, PHP_ONIG_BAD_KOI8_ENTRY). */
static OnigEncoding encoding(int id)
{
	switch (id) {
	case 0: return ONIG_ENCODING_EUC_JP;
	case 1: return ONIG_ENCODING_UTF8;
	case 2: return ONIG_ENCODING_UTF16_BE;
	case 3: return ONIG_ENCODING_UTF16_LE;
	case 4: return ONIG_ENCODING_UTF32_BE;
	case 5: return ONIG_ENCODING_UTF32_LE;
	case 6: return ONIG_ENCODING_SJIS;
	case 7: return ONIG_ENCODING_BIG5;
	case 8: return ONIG_ENCODING_EUC_CN;
	case 9: return ONIG_ENCODING_EUC_TW;
	case 10: return ONIG_ENCODING_EUC_KR;
	case 11: return ONIG_ENCODING_KOI8_R;
	case 12: return ONIG_ENCODING_ISO_8859_1;
	case 13: return ONIG_ENCODING_ISO_8859_2;
	case 14: return ONIG_ENCODING_ISO_8859_3;
	case 15: return ONIG_ENCODING_ISO_8859_4;
	case 16: return ONIG_ENCODING_ISO_8859_5;
	case 17: return ONIG_ENCODING_ISO_8859_6;
	case 18: return ONIG_ENCODING_ISO_8859_7;
	case 19: return ONIG_ENCODING_ISO_8859_8;
	case 20: return ONIG_ENCODING_ISO_8859_9;
	case 21: return ONIG_ENCODING_ISO_8859_10;
	case 22: return ONIG_ENCODING_ISO_8859_11;
	case 23: return ONIG_ENCODING_ISO_8859_13;
	case 24: return ONIG_ENCODING_ISO_8859_14;
	case 25: return ONIG_ENCODING_ISO_8859_15;
	case 26: return ONIG_ENCODING_ISO_8859_16;
	case 27: return ONIG_ENCODING_ASCII;
	}
	return ONIG_ENCODING_UNDEF;
}

/* In the order of mbregex.rs's Syntax. */
static OnigSyntaxType *syntax(int id)
{
	switch (id) {
	case 0: return ONIG_SYNTAX_JAVA;
	case 1: return ONIG_SYNTAX_GNU_REGEX;
	case 2: return ONIG_SYNTAX_GREP;
	case 3: return ONIG_SYNTAX_EMACS;
	case 4: return ONIG_SYNTAX_RUBY;
	case 5: return ONIG_SYNTAX_PERL;
	case 6: return ONIG_SYNTAX_POSIX_BASIC;
	case 7: return ONIG_SYNTAX_POSIX_EXTENDED;
	}
	return ONIG_SYNTAX_RUBY;
}

int rhp_onig_init(void)
{
	return onig_init();
}

/* onig_new; on failure the message (ONIG_MAX_ERROR_MESSAGE_LEN bytes) in err. */
int rhp_onig_new(OnigRegex *reg, const OnigUChar *pat, size_t len, OnigOptionType options,
	int enc, int syn, OnigUChar *err)
{
	OnigErrorInfo info;
	int r = onig_new(reg, pat, pat + len, options, encoding(enc), syntax(syn), &info);
	if (r != ONIG_NORMAL) {
		onig_error_code_to_str(err, r, &info);
	}
	return r;
}

/* onig_error_code_to_str without error info. */
void rhp_onig_error_str(int code, OnigUChar *err)
{
	onig_error_code_to_str(err, code);
}

static OnigMatchParam *match_param(int set_stack, unsigned int stack, int set_retry,
	unsigned int retry)
{
	OnigMatchParam *mp = onig_new_match_param();
	onig_initialize_match_param(mp);
	if (set_stack) {
		onig_set_match_stack_limit_size_of_match_param(mp, stack);
	}
	if (set_retry) {
		onig_set_retry_limit_in_match_of_match_param(mp, retry);
	}
	return mp;
}

/* _php_mb_onig_search over str[0..len] from start. */
int rhp_onig_search(OnigRegex reg, const OnigUChar *str, size_t len, size_t start,
	OnigRegion *region, int set_stack, unsigned int stack, int set_retry, unsigned int retry)
{
	OnigMatchParam *mp = match_param(set_stack, stack, set_retry, retry);
	int r = onig_search_with_param(reg, str, str + len, str + start, str + len, region,
		ONIG_OPTION_NONE, mp);
	onig_free_match_param(mp);
	return r;
}

/* mb_ereg_match's onig_match_with_param at the start of str. */
int rhp_onig_match(OnigRegex reg, const OnigUChar *str, size_t len, int set_stack,
	unsigned int stack, int set_retry, unsigned int retry)
{
	OnigMatchParam *mp = match_param(set_stack, stack, set_retry, retry);
	int r = onig_match_with_param(reg, str, str + len, str, NULL, ONIG_OPTION_NONE, mp);
	onig_free_match_param(mp);
	return r;
}

/* The region's registers: their count, and its beg/end arrays. */
int rhp_onig_region_regs(const OnigRegion *region, const int **beg, const int **end)
{
	*beg = region->beg;
	*end = region->end;
	return region->num_regs;
}

int rhp_onig_max_error_message_len(void)
{
	return ONIG_MAX_ERROR_MESSAGE_LEN;
}
