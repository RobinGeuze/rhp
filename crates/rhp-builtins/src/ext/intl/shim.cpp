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
#include <unicode/udat.h>
#include <unicode/ucal.h>
#include <unicode/utrans.h>
#include <unicode/uspoof.h>
#include <unicode/uchar.h>
#include <unicode/ubidi.h>
#include <unicode/uscript.h>
#include <unicode/timezone.h>
#include <unicode/brkiter.h>
#include <unicode/ucnv.h>
#include <unicode/ulistformatter.h>
#include <unicode/ures.h>
#include <unicode/ucnv_cb.h>

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
/* transliterator_class.h, breakiterator_iterators.h */
#define TRANSLITERATOR_FORWARD UTRANS_FORWARD
#define TRANSLITERATOR_REVERSE UTRANS_REVERSE
#define PARTS_ITERATOR_KEY_SEQUENTIAL 0
#define PARTS_ITERATOR_KEY_LEFT 1
#define PARTS_ITERATOR_KEY_RIGHT 2
/* listformatter_class.h */
#define INTL_LISTFORMATTER_FALLBACK_TYPE_AND 0
#define INTL_LISTFORMATTER_FALLBACK_WIDTH_WIDE 0

using icu::BreakIterator;
using icu::TimeZone;

extern "C" {

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

/* kind: 0 int, 1 string, 2 null, 3 float */
struct rhp_intl_class_const {
	const char *class_name;
	const char *name;
	int kind;
	long long value;
	const char *str;
	const char *attr;
	double dvalue;
};

#define RHP_INTL_CLASS_INT(c, name, value, attr) {c, name, 0, (long long) (value), NULL, attr, 0},
#define RHP_INTL_CLASS_STR(c, name, value, attr) {c, name, 1, 0, value, attr, 0},
#define RHP_INTL_CLASS_NULL(c, name, value, attr) {c, name, 2, 0, NULL, attr, 0},
#define RHP_INTL_CLASS_FLOAT(c, name, value, attr) {c, name, 3, 0, NULL, attr, (double) (value)},

static const struct rhp_intl_class_const rhp_intl_class_consts[] = {
#include "class_consts.h"
	{NULL, NULL, 0, 0, NULL, NULL, 0},
};

const struct rhp_intl_class_const *rhp_intl_class_constants(void) { return rhp_intl_class_consts; }

size_t rhp_intl_uidna_info_size(void) { return sizeof(UIDNAInfo); }

int rhp_intl_unum_format_symbol_count(void) { return UNUM_FORMAT_SYMBOL_COUNT; }

/* sys.rs */
static_assert(U_ZERO_ERROR == 0, "U_ZERO_ERROR");
static_assert(U_ILLEGAL_ARGUMENT_ERROR == 1, "U_ILLEGAL_ARGUMENT_ERROR");
static_assert(U_INVALID_CHAR_FOUND == 10, "U_INVALID_CHAR_FOUND");
static_assert(U_BUFFER_OVERFLOW_ERROR == 15, "U_BUFFER_OVERFLOW_ERROR");
static_assert(U_STRING_NOT_TERMINATED_WARNING == -124, "U_STRING_NOT_TERMINATED_WARNING");
static_assert(UBRK_CHARACTER == 0, "UBRK_CHARACTER");
static_assert(UBRK_DONE == -1, "UBRK_DONE");
static_assert(USEARCH_DONE == -1, "USEARCH_DONE");
static_assert(UCOL_STRENGTH == 5, "UCOL_STRENGTH");
static_assert(UCOL_SECONDARY == 1, "UCOL_SECONDARY");
static_assert(UCOL_EQUAL == 0, "UCOL_EQUAL");
static_assert(U_UNSUPPORTED_ERROR == 16, "U_UNSUPPORTED_ERROR");
static_assert(U_INTERNAL_PROGRAM_ERROR == 5, "U_INTERNAL_PROGRAM_ERROR");
static_assert(U_MEMORY_ALLOCATION_ERROR == 7, "U_MEMORY_ALLOCATION_ERROR");
static_assert(U_PATTERN_SYNTAX_ERROR == 65799, "U_PATTERN_SYNTAX_ERROR");
static_assert(ULOC_FULLNAME_CAPACITY == 157, "ULOC_FULLNAME_CAPACITY");
static_assert(ULOC_ACCEPT_FAILED == 0, "ULOC_ACCEPT_FAILED");
static_assert(ULOC_ACTUAL_LOCALE == 0 && ULOC_VALID_LOCALE == 1, "ULOC_*_LOCALE");
static_assert(sizeof(UParseError) == 72, "UParseError");
static_assert(offsetof(UParseError, preContext) == 8, "UParseError.preContext");
static_assert(sizeof(UBool) == 1, "UBool");
static_assert(sizeof(UChar) == 2, "UChar");
static_assert(sizeof(UIDNAInfo) == 16, "UIDNAInfo");
static_assert(offsetof(UIDNAInfo, isTransitionalDifferent) == 2, "UIDNAInfo.isTransitionalDifferent");
static_assert(offsetof(UIDNAInfo, errors) == 4, "UIDNAInfo.errors");
static_assert(U_INVALID_STATE_ERROR == 27, "U_INVALID_STATE_ERROR");
static_assert(U_USING_FALLBACK_WARNING == -128 && U_USING_DEFAULT_WARNING == -127, "U_USING_*_WARNING");
static_assert(U_AMBIGUOUS_ALIAS_WARNING == -122, "U_AMBIGUOUS_ALIAS_WARNING");
static_assert(URES_STRING == 0 && URES_BINARY == 1 && URES_TABLE == 2 && URES_INT == 7
	&& URES_ARRAY == 8 && URES_INT_VECTOR == 14, "URES_*");
static_assert(UCNV_UNASSIGNED == 0 && UCNV_ILLEGAL == 1 && UCNV_IRREGULAR == 2 && UCNV_RESET == 3
	&& UCNV_CLOSE == 4 && UCNV_CLONE == 5, "UConverterCallbackReason");
/* converter.rs's ToUArgs / FromUArgs */
static_assert(offsetof(UConverterToUnicodeArgs, source) == 2 * sizeof(void *), "ToUArgs.source");
static_assert(offsetof(UConverterToUnicodeArgs, target) == 4 * sizeof(void *), "ToUArgs.target");
static_assert(sizeof(UConverterToUnicodeArgs) == 7 * sizeof(void *), "ToUArgs");
static_assert(offsetof(UConverterFromUnicodeArgs, target) == 4 * sizeof(void *), "FromUArgs.target");
static_assert(sizeof(UConverterFromUnicodeArgs) == 7 * sizeof(void *), "FromUArgs");

/* PHP_MINFO_FUNCTION(intl)'s version macros: 0 ICU, 1 ICU data, 2 Unicode. */
const char *rhp_intl_info_version(int which)
{
	switch (which) {
		case 0: return U_ICU_VERSION;
#ifdef U_ICU_DATA_VERSION
		case 1: return U_ICU_DATA_VERSION;
#endif
		case 2: return U_UNICODE_VERSION;
		default: return NULL;
	}
}

} /* extern "C" */
