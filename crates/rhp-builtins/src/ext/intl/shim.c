/*
 * What intl needs from ICU's headers rather than its symbols: php's global
 * intl constants (consts.h) with the linked ICU's values, and checks that
 * the enum values and the struct layout sys.rs declares match the headers.
 */

#include <stddef.h>

#include <unicode/utypes.h>
#include <unicode/uvernum.h>
#include <unicode/ubrk.h>
#include <unicode/ucol.h>
#include <unicode/uidna.h>
#include <unicode/uloc.h>
#include <unicode/parseerr.h>
#include <unicode/usearch.h>
#include <unicode/unum.h>

/* php's own @cvalue macros (intl_data.h, grapheme.h, idn.h) */
#define INTL_MAX_LOCALE_LEN (ULOC_FULLNAME_CAPACITY - 1)
#define GRAPHEME_EXTRACT_TYPE_COUNT 0
#define GRAPHEME_EXTRACT_TYPE_MAXBYTES 1
#define GRAPHEME_EXTRACT_TYPE_MAXCHARS 2
#define INTL_IDN_VARIANT_UTS46 1
/* collator_sort.h, formatter_format.h, locale.h */
#define COLLATOR_SORT_REGULAR 0
#define COLLATOR_SORT_STRING 1
#define COLLATOR_SORT_NUMERIC 2
#define FORMAT_TYPE_DEFAULT 0
#define FORMAT_TYPE_INT32 1
#define FORMAT_TYPE_INT64 2
#define FORMAT_TYPE_DOUBLE 3
#define FORMAT_TYPE_CURRENCY 4
#define LOC_LANG_TAG "language"
#define LOC_SCRIPT_TAG "script"
#define LOC_REGION_TAG "region"
#define LOC_VARIANT_TAG "variant"
#define LOC_EXTLANG_TAG "extlang"
#define LOC_GRANDFATHERED_LANG_TAG "grandfathered"
#define LOC_PRIVATE_TAG "private"

struct rhp_intl_const {
	const char *name;
	long long value;
	const char *str;
};

#define RHP_INTL_INT(name, value) {name, (long long) (value), NULL},
#define RHP_INTL_STR(name, value) {name, 0, value},

static const struct rhp_intl_const rhp_intl_consts[] = {
#include "consts.h"
	{NULL, 0, NULL},
};

const struct rhp_intl_const *rhp_intl_constants(void) { return rhp_intl_consts; }

/* kind: 0 int, 1 string, 2 null */
struct rhp_intl_class_const {
	const char *class_name;
	const char *name;
	int kind;
	long long value;
	const char *str;
	const char *attr;
};

#define RHP_INTL_CLASS_INT(c, name, value, attr) {c, name, 0, (long long) (value), NULL, attr},
#define RHP_INTL_CLASS_STR(c, name, value, attr) {c, name, 1, 0, value, attr},
#define RHP_INTL_CLASS_NULL(c, name, value, attr) {c, name, 2, 0, NULL, attr},

static const struct rhp_intl_class_const rhp_intl_class_consts[] = {
#include "class_consts.h"
	{NULL, NULL, 0, 0, NULL, NULL},
};

const struct rhp_intl_class_const *rhp_intl_class_constants(void) { return rhp_intl_class_consts; }

size_t rhp_intl_uidna_info_size(void) { return sizeof(UIDNAInfo); }

int rhp_intl_unum_format_symbol_count(void) { return UNUM_FORMAT_SYMBOL_COUNT; }

/* sys.rs */
_Static_assert(U_ZERO_ERROR == 0, "U_ZERO_ERROR");
_Static_assert(U_ILLEGAL_ARGUMENT_ERROR == 1, "U_ILLEGAL_ARGUMENT_ERROR");
_Static_assert(U_INVALID_CHAR_FOUND == 10, "U_INVALID_CHAR_FOUND");
_Static_assert(U_BUFFER_OVERFLOW_ERROR == 15, "U_BUFFER_OVERFLOW_ERROR");
_Static_assert(U_STRING_NOT_TERMINATED_WARNING == -124, "U_STRING_NOT_TERMINATED_WARNING");
_Static_assert(UBRK_CHARACTER == 0, "UBRK_CHARACTER");
_Static_assert(UBRK_DONE == -1, "UBRK_DONE");
_Static_assert(USEARCH_DONE == -1, "USEARCH_DONE");
_Static_assert(UCOL_STRENGTH == 5, "UCOL_STRENGTH");
_Static_assert(UCOL_SECONDARY == 1, "UCOL_SECONDARY");
_Static_assert(UCOL_EQUAL == 0, "UCOL_EQUAL");
_Static_assert(U_UNSUPPORTED_ERROR == 16, "U_UNSUPPORTED_ERROR");
_Static_assert(ULOC_FULLNAME_CAPACITY == 157, "ULOC_FULLNAME_CAPACITY");
_Static_assert(ULOC_ACCEPT_FAILED == 0, "ULOC_ACCEPT_FAILED");
_Static_assert(sizeof(UParseError) == 72, "UParseError");
_Static_assert(offsetof(UParseError, preContext) == 8, "UParseError.preContext");
_Static_assert(sizeof(UBool) == 1, "UBool");
_Static_assert(sizeof(UChar) == 2, "UChar");
_Static_assert(sizeof(UIDNAInfo) == 16, "UIDNAInfo");
_Static_assert(offsetof(UIDNAInfo, isTransitionalDifferent) == 2, "UIDNAInfo.isTransitionalDifferent");
_Static_assert(offsetof(UIDNAInfo, errors) == 4, "UIDNAInfo.errors");
