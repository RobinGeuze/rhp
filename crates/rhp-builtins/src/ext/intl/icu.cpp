/*
 * The ICU C++ that intl needs and ICU's C API does not offer, as small C
 * functions for the Rust side: MessageFormat's argument types, formatting
 * with named arguments and parsing (php's msgformat_helpers.cpp), and the
 * TimeZone and Calendar objects (php's timezone/ and calendar/). The PHP
 * side of each (argument conversion, errors) stays in Rust.
 *
 * Objects cross as `void *`: a `TimeZone *` or `Calendar *` the caller
 * owns and frees with the matching `_delete`. Strings come back malloc'd
 * (free with rhp_intl_free).
 */

#include <stdlib.h>
#include <string.h>

#include <memory>
#include <vector>

#include <unicode/basictz.h>
#include <unicode/calendar.h>
#include <unicode/datefmt.h>
#include <unicode/gregocal.h>
#include <unicode/locid.h>
#include <unicode/messagepattern.h>
#include <unicode/msgfmt.h>
#include <unicode/strenum.h>
#include <unicode/timezone.h>
#include <unicode/ucal.h>
#include <unicode/udat.h>
#include <unicode/uenum.h>
#include <unicode/umsg.h>
#include <unicode/ustring.h>

U_NAMESPACE_BEGIN
/* ICU declares MessageFormatAdapter a friend of MessageFormat, as php
 * uses it: a way to its argument types and pattern. */
class MessageFormatAdapter {
public:
	static const Formattable::Type *phpGetArgTypeList(const MessageFormat &m, int32_t &count) {
		return m.getArgTypeList(count);
	}
	static const MessagePattern phpGetMessagePattern(MessageFormat *m) { return m->msgPattern; }
};
U_NAMESPACE_END

using icu::BasicTimeZone;
using icu::Calendar;
using icu::DateFormat;
using icu::Format;
using icu::Formattable;
using icu::GregorianCalendar;
using icu::Locale;
using icu::MessageFormat;
using icu::MessageFormatAdapter;
using icu::MessagePattern;
using icu::StringEnumeration;
using icu::TimeZone;
using icu::UnicodeString;

static_assert(Formattable::kDate == 0, "kDate");
static_assert(Formattable::kDouble == 1, "kDouble");
static_assert(Formattable::kLong == 2, "kLong");
static_assert(Formattable::kString == 3, "kString");
static_assert(Formattable::kArray == 4, "kArray");
static_assert(Formattable::kInt64 == 5, "kInt64");
static_assert(Formattable::kObject == 6, "kObject");

static UChar *copy_ustr(const UnicodeString &s, int32_t *len) {
	*len = s.length();
	UChar *p = (UChar *) malloc(sizeof(UChar) * (*len + 1));
	if (p) {
		memcpy(p, s.getBuffer(), sizeof(UChar) * *len);
		p[*len] = 0;
	}
	return p;
}

extern "C" {

void rhp_intl_free(void *p) { free(p); }

/* --- MessageFormat (msgformat_helpers.cpp) ------------------------------ */

/* One argument's type: by number (name NULL) or by name. */
struct rhp_msg_type {
	int32_t number;
	UChar *name;
	int32_t name_len;
	int32_t type;
};

int32_t rhp_msgfmt_uses_named(const void *fmt) {
	return ((const MessageFormat *) fmt)->usesNamedArguments() ? 1 : 0;
}

/* `umsg_format_arg_count`. */
int32_t rhp_msgfmt_arg_count(const void *fmt) {
	int32_t n = 0;
	MessageFormatAdapter::phpGetArgTypeList(*(const MessageFormat *) fmt, n);
	return n;
}

static void push_type(std::vector<rhp_msg_type> &out, int32_t number, const UnicodeString *name, int32_t type) {
	rhp_msg_type t;
	t.number = number;
	t.name = NULL;
	t.name_len = 0;
	if (name) {
		t.name = copy_ustr(*name, &t.name_len);
	}
	t.type = type;
	out.push_back(t);
}

/* `umsg_get_numeric_types` / `umsg_parse_format`: the arguments' types.
 * The count, or -1 with `code` and `msg` (php's intl_errors_set). */
int32_t rhp_msgfmt_arg_types(void *fmt, rhp_msg_type **out, int32_t *code, const char **msg) {
	MessageFormat *mf = (MessageFormat *) fmt;
	std::vector<rhp_msg_type> types;
	*code = U_ZERO_ERROR;
	*msg = NULL;
	if (!mf->usesNamedArguments()) {
		int32_t n = 0;
		const Formattable::Type *t = MessageFormatAdapter::phpGetArgTypeList(*mf, n);
		for (int32_t i = 0; i < n; i++) {
			push_type(types, i, NULL, t[i]);
		}
	} else {
		const MessagePattern mp = MessageFormatAdapter::phpGetMessagePattern(mf);
		int32_t parts_count = mp.countParts();
		for (int32_t i = 0; i < parts_count - 2 && U_SUCCESS((UErrorCode) *code); i++) {
			MessagePattern::Part p = mp.getPart(i);
			if (p.getType() != UMSGPAT_PART_TYPE_ARG_START) {
				continue;
			}
			MessagePattern::Part name_part = mp.getPart(++i);
			int32_t number = -1;
			UnicodeString argName;
			if (name_part.getType() == UMSGPAT_PART_TYPE_ARG_NAME) {
				argName = mp.getSubstring(name_part);
			} else if (name_part.getType() == UMSGPAT_PART_TYPE_ARG_NUMBER) {
				number = name_part.getValue();
				if (number < 0) {
					*code = U_INVALID_FORMAT_ERROR;
					*msg = "Found part with negative number";
					continue;
				}
			} else {
				*code = U_INVALID_FORMAT_ERROR;
				*msg = "Invalid part type encountered";
				continue;
			}
			rhp_msg_type *stored = NULL;
			for (auto &t : types) {
				bool same = number >= 0
					? (t.name == NULL && t.number == number)
					: (t.name != NULL && argName == UnicodeString(t.name, t.name_len));
				if (same) {
					stored = &t;
					break;
				}
			}
			if (!stored) {
				push_type(types, number, number >= 0 ? NULL : &argName, Formattable::kObject);
				stored = &types.back();
			}
			Formattable::Type type = Formattable::kString;
			UMessagePatternArgType argType = p.getArgType();
			if (argType == UMSGPAT_ARG_TYPE_NONE) {
				type = Formattable::kString;
			} else if (argType == UMSGPAT_ARG_TYPE_SIMPLE) {
				MessagePattern::Part type_part = mp.getPart(++i);
				if (type_part.getType() == UMSGPAT_PART_TYPE_ARG_TYPE) {
					UnicodeString typeString = mp.getSubstring(type_part);
#define ASCII_LITERAL(s) UNICODE_STRING(s, sizeof(s)-1)
					if (typeString == ASCII_LITERAL("number")) {
						MessagePattern::Part style_part = mp.getPart(i + 1);
						if (style_part.getType() == UMSGPAT_PART_TYPE_ARG_STYLE) {
							UnicodeString styleString = mp.getSubstring(style_part);
							if (styleString == ASCII_LITERAL("integer")) {
								type = Formattable::kInt64;
							} else {
								type = Formattable::kDouble;
							}
						} else {
							type = Formattable::kDouble;
						}
					} else if ((typeString == ASCII_LITERAL("date")) || (typeString == ASCII_LITERAL("time"))) {
						type = Formattable::kDate;
					} else if ((typeString == ASCII_LITERAL("spellout")) || (typeString == ASCII_LITERAL("ordinal"))
							|| (typeString == ASCII_LITERAL("duration"))) {
						type = Formattable::kDouble;
					}
#undef ASCII_LITERAL
				} else {
					*code = U_PARSE_ERROR;
					*msg = "Expected UMSGPAT_PART_TYPE_ARG_TYPE part following UMSGPAT_ARG_TYPE_SIMPLE part";
					continue;
				}
			} else if (argType == UMSGPAT_ARG_TYPE_PLURAL || argType == UMSGPAT_ARG_TYPE_CHOICE
					|| argType == UMSGPAT_ARG_TYPE_SELECTORDINAL) {
				type = Formattable::kDouble;
			} else {
				type = Formattable::kString;
			}
			if (stored->type != Formattable::kObject && stored->type != type) {
				*code = U_ARGUMENT_TYPE_MISMATCH;
				*msg = "Inconsistent types declared for an argument";
				continue;
			}
			stored->type = type;
		}
	}
	int32_t n = (int32_t) types.size();
	*out = (rhp_msg_type *) malloc(sizeof(rhp_msg_type) * (n ? n : 1));
	if (n) {
		memcpy(*out, types.data(), sizeof(rhp_msg_type) * n);
	}
	if (U_FAILURE((UErrorCode) *code)) {
		return -1;
	}
	return n;
}

void rhp_msgfmt_free_types(rhp_msg_type *t, int32_t n) {
	for (int32_t i = 0; i < n; i++) {
		free(t[i].name);
	}
	free(t);
}

/* Whether the pattern has date subformats `umsg_set_timezone` would set. */
int32_t rhp_msgfmt_has_date_formats(void *fmt) {
	int32_t count = 0;
	const Format **formats = ((MessageFormat *) fmt)->getFormats(count);
	if (formats == NULL) {
		return -1;
	}
	int32_t any = 0;
	for (int i = 0; i < count; i++) {
		if (dynamic_cast<const DateFormat *>(formats[i]) != NULL) {
			any = 1;
		}
	}
	/* php does not free the array either: ICU allocates it with uprv_malloc. */
	return any;
}

/* `umsg_set_timezone`: gives each date subformat a clone of `tz`. */
void rhp_msgfmt_set_timezone(void *fmt, const void *tz) {
	MessageFormat *mf = (MessageFormat *) fmt;
	int32_t count = 0;
	const Format **formats = mf->getFormats(count);
	if (formats == NULL) {
		return;
	}
	for (int i = 0; i < count; i++) {
		DateFormat *df = dynamic_cast<DateFormat *>(const_cast<Format *>(formats[i]));
		if (df != NULL) {
			df->adoptTimeZone(((const TimeZone *) tz)->clone());
		}
	}
}

/* One argument for `format`: its key and its value as `type`. */
struct rhp_msg_arg {
	const UChar *name;
	int32_t name_len;
	int32_t type;
	double d;
	int64_t i;
	const UChar *s;
	int32_t s_len;
};

/* `MessageFormat::format` with named keys; the result malloc'd. */
UChar *rhp_msgfmt_format(void *fmt, const rhp_msg_arg *args, int32_t n, int32_t *out_len, UErrorCode *status) {
	std::vector<Formattable> fargs(n);
	std::vector<UnicodeString> names(n);
	for (int32_t k = 0; k < n; k++) {
		names[k] = UnicodeString(args[k].name, args[k].name_len);
		switch (args[k].type) {
		case Formattable::kString:
			fargs[k].adoptString(new UnicodeString(args[k].s, args[k].s_len));
			break;
		case Formattable::kDouble:
			fargs[k].setDouble(args[k].d);
			break;
		case Formattable::kLong:
			fargs[k].setLong((int32_t) args[k].i);
			break;
		case Formattable::kInt64:
			fargs[k].setInt64(args[k].i);
			break;
		case Formattable::kDate:
			fargs[k].setDate(args[k].d);
			break;
		}
	}
	UnicodeString result;
	((MessageFormat *) fmt)->format(names.empty() ? NULL : &names[0],
		fargs.empty() ? NULL : &fargs[0], n, result, *status);
	if (U_FAILURE(*status)) {
		return NULL;
	}
	return copy_ustr(result, out_len);
}

/* One parsed value. */
struct rhp_msg_parsed {
	int32_t type;
	double d;
	int64_t i;
	UChar *s;
	int32_t s_len;
};

/* `MessageFormat::parse` (umsg_parse_helper); the array malloc'd, each
 * string too. */
rhp_msg_parsed *rhp_msgfmt_parse(const void *fmt, const UChar *src, int32_t len, int32_t *count, UErrorCode *status) {
	UnicodeString s(src, len);
	*count = 0;
	Formattable *fargs = ((const MessageFormat *) fmt)->parse(s, *count, *status);
	if (U_FAILURE(*status)) {
		return NULL;
	}
	rhp_msg_parsed *out = (rhp_msg_parsed *) calloc(*count ? *count : 1, sizeof(rhp_msg_parsed));
	for (int32_t k = 0; k < *count; k++) {
		out[k].type = fargs[k].getType();
		switch (fargs[k].getType()) {
		case Formattable::kDate:
			out[k].d = (double) fargs[k].getDate();
			break;
		case Formattable::kDouble:
			out[k].d = fargs[k].getDouble();
			break;
		case Formattable::kLong:
			out[k].i = fargs[k].getLong();
			break;
		case Formattable::kInt64:
			out[k].i = fargs[k].getInt64();
			break;
		case Formattable::kString: {
			UnicodeString tmp;
			fargs[k].getString(tmp);
			out[k].s = copy_ustr(tmp, &out[k].s_len);
			break;
		}
		default:
			break;
		}
	}
	delete[] fargs;
	return out;
}

/* --- TimeZone (timezone/) ------------------------------------------- */

static UChar *tz_id_of(const UnicodeString &s, int32_t *len) { return copy_ustr(s, len); }

void *rhp_tz_create(const UChar *id, int32_t len) {
	return TimeZone::createTimeZone(UnicodeString(id, len));
}
void *rhp_tz_create_default(void) { return TimeZone::createDefault(); }
const void *rhp_tz_gmt(void) { return TimeZone::getGMT(); }
const void *rhp_tz_unknown(void) { return &TimeZone::getUnknown(); }
void *rhp_tz_clone(const void *tz) { return ((const TimeZone *) tz)->clone(); }
void rhp_tz_delete(void *tz) { delete (TimeZone *) tz; }
int32_t rhp_tz_is_unknown(const void *tz) { return *(const TimeZone *) tz == TimeZone::getUnknown(); }
int32_t rhp_tz_equals(const void *a, const void *b) { return *(const TimeZone *) a == *(const TimeZone *) b; }
UChar *rhp_tz_get_id(const void *tz, int32_t *len) {
	UnicodeString id;
	((const TimeZone *) tz)->getID(id);
	if (id.isBogus()) {
		*len = -1;
		return NULL;
	}
	return tz_id_of(id, len);
}
int32_t rhp_tz_raw_offset(const void *tz) { return ((const TimeZone *) tz)->getRawOffset(); }
int32_t rhp_tz_use_daylight(const void *tz) { return ((const TimeZone *) tz)->useDaylightTime(); }
int32_t rhp_tz_dst_savings(const void *tz) { return ((const TimeZone *) tz)->getDSTSavings(); }
int32_t rhp_tz_has_same_rules(const void *a, const void *b) {
	return ((const TimeZone *) a)->hasSameRules(*(const TimeZone *) b);
}
void rhp_tz_get_offset(const void *tz, double date, int32_t local, int32_t *raw, int32_t *dst, UErrorCode *status) {
	((const TimeZone *) tz)->getOffset((UDate) date, (UBool) local, *raw, *dst, *status);
}
UChar *rhp_tz_display_name(const void *tz, int32_t daylight, int32_t type, const char *locale, int32_t *len) {
	UnicodeString result;
	((const TimeZone *) tz)->getDisplayName((UBool) daylight, (TimeZone::EDisplayType) type,
		Locale::createFromName(locale), result);
	return copy_ustr(result, len);
}
double rhp_cal_now(void) { return Calendar::getNow(); }

int32_t rhp_tz_count_equivalent(const UChar *id, int32_t len) {
	return TimeZone::countEquivalentIDs(UnicodeString(id, len));
}
UChar *rhp_tz_equivalent_id(const UChar *id, int32_t len, int32_t index, int32_t *out_len) {
	const UnicodeString r = TimeZone::getEquivalentID(UnicodeString(id, len), index);
	return copy_ustr(r, out_len);
}
UChar *rhp_tz_canonical_id(const UChar *id, int32_t len, int32_t *is_system, int32_t *out_len, UErrorCode *status) {
	UnicodeString result;
	UBool sys = 0;
	TimeZone::getCanonicalID(UnicodeString(id, len), result, sys, *status);
	*is_system = sys;
	if (U_FAILURE(*status)) {
		return NULL;
	}
	return copy_ustr(result, out_len);
}
int32_t rhp_tz_region(const UChar *id, int32_t len, char *out, int32_t cap, UErrorCode *status) {
	return TimeZone::getRegion(UnicodeString(id, len), out, cap, *status);
}
const char *rhp_tz_data_version(UErrorCode *status) { return TimeZone::getTZDataVersion(*status); }
UChar *rhp_tz_iana_id(const UChar *id, int32_t len, int32_t *out_len, UErrorCode *status) {
	UnicodeString result;
#if U_ICU_VERSION_MAJOR_NUM >= 74
	TimeZone::getIanaID(UnicodeString(id, len), result, *status);
#else
	*status = U_UNSUPPORTED_ERROR;
#endif
	if (U_FAILURE(*status)) {
		return NULL;
	}
	return copy_ustr(result, out_len);
}
UChar *rhp_tz_windows_id(const UChar *id, int32_t len, int32_t *out_len, UErrorCode *status) {
	UnicodeString result;
	TimeZone::getWindowsID(UnicodeString(id, len), result, *status);
	if (U_FAILURE(*status)) {
		return NULL;
	}
	return copy_ustr(result, out_len);
}
UChar *rhp_tz_id_for_windows(const UChar *id, int32_t len, const char *region, int32_t *out_len, UErrorCode *status) {
	UnicodeString result;
	TimeZone::getIDForWindowsID(UnicodeString(id, len), region, result, *status);
	if (U_FAILURE(*status)) {
		return NULL;
	}
	return copy_ustr(result, out_len);
}

static UEnumeration *to_uenum(StringEnumeration *se) {
	if (se == NULL) {
		return NULL;
	}
	UErrorCode status = U_ZERO_ERROR;
	return uenum_openFromStringEnumeration(se, &status);
}
/* kind 0: all, 1: country (arg s), 2: raw offset (arg n) */
UEnumeration *rhp_tz_enum(int32_t kind, const char *s, int32_t n) {
	switch (kind) {
	case 0: return to_uenum(TimeZone::createEnumeration());
	case 1: return to_uenum(TimeZone::createEnumeration(s));
	default: return to_uenum(TimeZone::createEnumeration(n));
	}
}
UEnumeration *rhp_tz_enum_ids(int32_t type, const char *region, const int32_t *offset, UErrorCode *status) {
	StringEnumeration *se = TimeZone::createTimeZoneIDEnumeration((USystemTimeZoneType) type, region, offset, *status);
	if (U_FAILURE(*status)) {
		delete se;
		return NULL;
	}
	return to_uenum(se);
}

} /* extern "C" */
