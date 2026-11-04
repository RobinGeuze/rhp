/*
 * Around the vendored timelib: php's estrndup for timelib_config.h, and the
 * struct layouts src/timelib.rs mirrors.
 */
#include <string.h>

#include "timelib.h"
#include "timelib_private.h"

/* zend_strndup: exactly `length` bytes, then a NUL. */
char *rhp_timelib_strndup(const char *s, size_t length)
{
	char *p = rhp_csrc_emalloc(length + 1);
	memcpy(p, s, length);
	p[length] = '\0';
	return p;
}

const char *rhp_timelib_ascii_version(void)
{
	return TIMELIB_ASCII_VERSION;
}

#define L(type, field) offsetof(type, field)
#define S(type) sizeof(type)

/* In the order src/timelib.rs's layout test lists them. */
static const size_t layout[] = {
	S(timelib_time),
	L(timelib_time, y), L(timelib_time, us), L(timelib_time, z),
	L(timelib_time, tz_abbr), L(timelib_time, tz_info), L(timelib_time, dst),
	L(timelib_time, relative), L(timelib_time, sse), L(timelib_time, have_time),
	L(timelib_time, have_weeknr_day), L(timelib_time, sse_uptodate),
	L(timelib_time, is_localtime), L(timelib_time, zone_type),
	S(timelib_rel_time),
	L(timelib_rel_time, y), L(timelib_rel_time, us), L(timelib_rel_time, weekday),
	L(timelib_rel_time, weekday_behavior), L(timelib_rel_time, first_last_day_of),
	L(timelib_rel_time, invert), L(timelib_rel_time, days),
	L(timelib_rel_time, special.type), L(timelib_rel_time, special.amount),
	L(timelib_rel_time, have_weekday_relative), L(timelib_rel_time, have_special_relative),
	S(timelib_tzinfo),
	L(timelib_tzinfo, name), L(timelib_tzinfo, _bit32), L(timelib_tzinfo, bit64),
	L(timelib_tzinfo, bit64.timecnt), L(timelib_tzinfo, bit64.typecnt),
	L(timelib_tzinfo, trans), L(timelib_tzinfo, trans_idx), L(timelib_tzinfo, type),
	L(timelib_tzinfo, timezone_abbr), L(timelib_tzinfo, leap_times), L(timelib_tzinfo, bc),
	L(timelib_tzinfo, location), L(timelib_tzinfo, posix_string), L(timelib_tzinfo, posix_info),
	S(tlocinfo),
	L(tlocinfo, country_code), L(tlocinfo, latitude), L(tlocinfo, longitude),
	L(tlocinfo, comments),
	S(ttinfo),
	L(ttinfo, offset), L(ttinfo, isdst), L(ttinfo, abbr_idx), L(ttinfo, isstdcnt),
	L(ttinfo, isgmtcnt),
	S(timelib_time_offset),
	L(timelib_time_offset, offset), L(timelib_time_offset, leap_secs),
	L(timelib_time_offset, is_dst), L(timelib_time_offset, abbr),
	L(timelib_time_offset, transition_time),
	S(timelib_error_message),
	L(timelib_error_message, error_code), L(timelib_error_message, position),
	L(timelib_error_message, character), L(timelib_error_message, message),
	S(timelib_error_container),
	L(timelib_error_container, error_messages), L(timelib_error_container, warning_messages),
	L(timelib_error_container, error_count), L(timelib_error_container, warning_count),
	S(timelib_tz_lookup_table),
	L(timelib_tz_lookup_table, name), L(timelib_tz_lookup_table, type),
	L(timelib_tz_lookup_table, gmtoffset), L(timelib_tz_lookup_table, full_tz_name),
	S(timelib_tzdb_index_entry),
	L(timelib_tzdb_index_entry, id), L(timelib_tzdb_index_entry, pos),
	S(timelib_tzdb),
	L(timelib_tzdb, version), L(timelib_tzdb, index_size), L(timelib_tzdb, index),
	L(timelib_tzdb, data),
	S(timelib_abbr_info),
	L(timelib_abbr_info, utc_offset), L(timelib_abbr_info, abbr), L(timelib_abbr_info, dst),
	S(timelib_posix_transitions),
	L(timelib_posix_transitions, count), L(timelib_posix_transitions, times),
	L(timelib_posix_transitions, types),
	S(timelib_posix_str),
	L(timelib_posix_str, dst_offset), L(timelib_posix_str, dst_begin),
	L(timelib_posix_str, dst_end),
};

const size_t *rhp_timelib_layout(size_t *n)
{
	*n = sizeof(layout) / sizeof(layout[0]);
	return layout;
}
