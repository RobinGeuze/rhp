// @generated from php-src *.stub.php by the stub-param extractor; do not edit.

/// A builtin parameter whose declared type is made of scalar /
/// array types only: (position, name, declared type, variadic, nullable,
/// accepts string, accepts array, by reference).
pub type ParamInfo = (u8, &'static str, &'static str, bool, bool, bool, bool, bool);

/// Per builtin (sorted by name), its scalar / array typed parameters.
pub static PARAMS: &[(&str, &[ParamInfo])] = &[
    (
        "_",
        &[(0, "message", "string", false, false, true, false, false)],
    ),
    (
        "abs",
        &[(0, "num", "int|float", false, false, false, false, false)],
    ),
    (
        "acos",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "acosh",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "addcslashes",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "characters", "string", false, false, true, false, false),
        ],
    ),
    (
        "addslashes",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "array_all",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_any",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_change_key_case",
        &[
            (0, "array", "array", false, false, false, true, false),
            (1, "case", "int", false, false, false, false, false),
        ],
    ),
    (
        "array_chunk",
        &[
            (0, "array", "array", false, false, false, true, false),
            (1, "length", "int", false, false, false, false, false),
            (
                2,
                "preserve_keys",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "array_column",
        &[
            (0, "array", "array", false, false, false, true, false),
            (
                1,
                "column_key",
                "string|int|null",
                false,
                true,
                true,
                false,
                false,
            ),
            (
                2,
                "index_key",
                "string|int|null",
                false,
                true,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "array_combine",
        &[
            (0, "keys", "array", false, false, false, true, false),
            (1, "values", "array", false, false, false, true, false),
        ],
    ),
    (
        "array_count_values",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_diff",
        &[
            (0, "array", "array", false, false, false, true, false),
            (1, "arrays", "array", true, false, false, true, false),
        ],
    ),
    (
        "array_diff_assoc",
        &[
            (0, "array", "array", false, false, false, true, false),
            (1, "arrays", "array", true, false, false, true, false),
        ],
    ),
    (
        "array_diff_key",
        &[
            (0, "array", "array", false, false, false, true, false),
            (1, "arrays", "array", true, false, false, true, false),
        ],
    ),
    (
        "array_diff_uassoc",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_diff_ukey",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_fill",
        &[
            (0, "start_index", "int", false, false, false, false, false),
            (1, "count", "int", false, false, false, false, false),
        ],
    ),
    (
        "array_fill_keys",
        &[(0, "keys", "array", false, false, false, true, false)],
    ),
    (
        "array_filter",
        &[
            (0, "array", "array", false, false, false, true, false),
            (2, "mode", "int", false, false, false, false, false),
        ],
    ),
    (
        "array_find",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_find_key",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_first",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_flip",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_intersect",
        &[
            (0, "array", "array", false, false, false, true, false),
            (1, "arrays", "array", true, false, false, true, false),
        ],
    ),
    (
        "array_intersect_assoc",
        &[
            (0, "array", "array", false, false, false, true, false),
            (1, "arrays", "array", true, false, false, true, false),
        ],
    ),
    (
        "array_intersect_key",
        &[
            (0, "array", "array", false, false, false, true, false),
            (1, "arrays", "array", true, false, false, true, false),
        ],
    ),
    (
        "array_intersect_uassoc",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_intersect_ukey",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_is_list",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_key_exists",
        &[(1, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_key_first",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_key_last",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_keys",
        &[
            (0, "array", "array", false, false, false, true, false),
            (2, "strict", "bool", false, false, false, false, false),
        ],
    ),
    (
        "array_last",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_map",
        &[
            (1, "array", "array", false, false, false, true, false),
            (2, "arrays", "array", true, false, false, true, false),
        ],
    ),
    (
        "array_merge",
        &[(0, "arrays", "array", true, false, false, true, false)],
    ),
    (
        "array_merge_recursive",
        &[(0, "arrays", "array", true, false, false, true, false)],
    ),
    (
        "array_pad",
        &[
            (0, "array", "array", false, false, false, true, false),
            (1, "length", "int", false, false, false, false, false),
        ],
    ),
    (
        "array_pop",
        &[(0, "array", "array", false, false, false, true, true)],
    ),
    (
        "array_product",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_push",
        &[(0, "array", "array", false, false, false, true, true)],
    ),
    (
        "array_rand",
        &[
            (0, "array", "array", false, false, false, true, false),
            (1, "num", "int", false, false, false, false, false),
        ],
    ),
    (
        "array_reduce",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_replace",
        &[
            (0, "array", "array", false, false, false, true, false),
            (1, "replacements", "array", true, false, false, true, false),
        ],
    ),
    (
        "array_replace_recursive",
        &[
            (0, "array", "array", false, false, false, true, false),
            (1, "replacements", "array", true, false, false, true, false),
        ],
    ),
    (
        "array_reverse",
        &[
            (0, "array", "array", false, false, false, true, false),
            (
                1,
                "preserve_keys",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "array_search",
        &[
            (1, "haystack", "array", false, false, false, true, false),
            (2, "strict", "bool", false, false, false, false, false),
        ],
    ),
    (
        "array_shift",
        &[(0, "array", "array", false, false, false, true, true)],
    ),
    (
        "array_slice",
        &[
            (0, "array", "array", false, false, false, true, false),
            (1, "offset", "int", false, false, false, false, false),
            (2, "length", "?int", false, true, false, false, false),
            (
                3,
                "preserve_keys",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "array_splice",
        &[
            (0, "array", "array", false, false, false, true, true),
            (1, "offset", "int", false, false, false, false, false),
            (2, "length", "?int", false, true, false, false, false),
        ],
    ),
    (
        "array_sum",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_udiff",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_udiff_assoc",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_udiff_uassoc",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_uintersect",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_uintersect_assoc",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_uintersect_uassoc",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "array_unique",
        &[
            (0, "array", "array", false, false, false, true, false),
            (1, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "array_unshift",
        &[(0, "array", "array", false, false, false, true, true)],
    ),
    (
        "array_values",
        &[(0, "array", "array", false, false, false, true, false)],
    ),
    (
        "arsort",
        &[
            (0, "array", "array", false, false, false, true, true),
            (1, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "asin",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "asinh",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "asort",
        &[
            (0, "array", "array", false, false, false, true, true),
            (1, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "assert_options",
        &[(0, "option", "int", false, false, false, false, false)],
    ),
    (
        "atan",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "atan2",
        &[
            (0, "y", "float", false, false, false, false, false),
            (1, "x", "float", false, false, false, false, false),
        ],
    ),
    (
        "atanh",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "base64_decode",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "strict", "bool", false, false, false, false, false),
        ],
    ),
    (
        "base64_encode",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "base_convert",
        &[
            (0, "num", "string", false, false, true, false, false),
            (1, "from_base", "int", false, false, false, false, false),
            (2, "to_base", "int", false, false, false, false, false),
        ],
    ),
    (
        "basename",
        &[
            (0, "path", "string", false, false, true, false, false),
            (1, "suffix", "string", false, false, true, false, false),
        ],
    ),
    (
        "bin2hex",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "bind_textdomain_codeset",
        &[
            (0, "domain", "string", false, false, true, false, false),
            (1, "codeset", "?string", false, true, true, false, false),
        ],
    ),
    (
        "bindec",
        &[(
            0,
            "binary_string",
            "string",
            false,
            false,
            true,
            false,
            false,
        )],
    ),
    (
        "bindtextdomain",
        &[
            (0, "domain", "string", false, false, true, false, false),
            (1, "directory", "?string", false, true, true, false, false),
        ],
    ),
    (
        "bzcompress",
        &[
            (0, "data", "string", false, false, true, false, false),
            (1, "block_size", "int", false, false, false, false, false),
            (2, "work_factor", "int", false, false, false, false, false),
        ],
    ),
    (
        "bzdecompress",
        &[
            (0, "data", "string", false, false, true, false, false),
            (
                1,
                "use_less_memory",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "bzread",
        &[(1, "length", "int", false, false, false, false, false)],
    ),
    (
        "bzwrite",
        &[
            (1, "data", "string", false, false, true, false, false),
            (2, "length", "?int", false, true, false, false, false),
        ],
    ),
    (
        "cal_days_in_month",
        &[
            (0, "calendar", "int", false, false, false, false, false),
            (1, "month", "int", false, false, false, false, false),
            (2, "year", "int", false, false, false, false, false),
        ],
    ),
    (
        "cal_from_jd",
        &[
            (0, "julian_day", "int", false, false, false, false, false),
            (1, "calendar", "int", false, false, false, false, false),
        ],
    ),
    (
        "cal_info",
        &[(0, "calendar", "int", false, false, false, false, false)],
    ),
    (
        "cal_to_jd",
        &[
            (0, "calendar", "int", false, false, false, false, false),
            (1, "month", "int", false, false, false, false, false),
            (2, "day", "int", false, false, false, false, false),
            (3, "year", "int", false, false, false, false, false),
        ],
    ),
    (
        "call_user_func_array",
        &[(1, "args", "array", false, false, false, true, false)],
    ),
    (
        "ceil",
        &[(0, "num", "int|float", false, false, false, false, false)],
    ),
    (
        "chdir",
        &[(0, "directory", "string", false, false, true, false, false)],
    ),
    (
        "checkdate",
        &[
            (0, "month", "int", false, false, false, false, false),
            (1, "day", "int", false, false, false, false, false),
            (2, "year", "int", false, false, false, false, false),
        ],
    ),
    (
        "checkdnsrr",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (1, "type", "string", false, false, true, false, false),
        ],
    ),
    (
        "chgrp",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "group", "string|int", false, false, true, false, false),
        ],
    ),
    (
        "chmod",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "permissions", "int", false, false, false, false, false),
        ],
    ),
    (
        "chop",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "characters", "string", false, false, true, false, false),
        ],
    ),
    (
        "chown",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "user", "string|int", false, false, true, false, false),
        ],
    ),
    (
        "chr",
        &[(0, "codepoint", "int", false, false, false, false, false)],
    ),
    (
        "chroot",
        &[(0, "directory", "string", false, false, true, false, false)],
    ),
    (
        "chunk_split",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "length", "int", false, false, false, false, false),
            (2, "separator", "string", false, false, true, false, false),
        ],
    ),
    (
        "class_alias",
        &[
            (0, "class", "string", false, false, true, false, false),
            (1, "alias", "string", false, false, true, false, false),
            (2, "autoload", "bool", false, false, false, false, false),
        ],
    ),
    (
        "class_exists",
        &[
            (0, "class", "string", false, false, true, false, false),
            (1, "autoload", "bool", false, false, false, false, false),
        ],
    ),
    (
        "class_implements",
        &[(1, "autoload", "bool", false, false, false, false, false)],
    ),
    (
        "class_parents",
        &[(1, "autoload", "bool", false, false, false, false, false)],
    ),
    (
        "class_uses",
        &[(1, "autoload", "bool", false, false, false, false, false)],
    ),
    (
        "clearstatcache",
        &[
            (
                0,
                "clear_realpath_cache",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
            (1, "filename", "string", false, false, true, false, false),
        ],
    ),
    (
        "clone",
        &[(
            1,
            "withProperties",
            "array",
            false,
            false,
            false,
            true,
            false,
        )],
    ),
    (
        "collator_asort",
        &[
            (1, "array", "array", false, false, false, true, true),
            (2, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "collator_compare",
        &[
            (1, "string1", "string", false, false, true, false, false),
            (2, "string2", "string", false, false, true, false, false),
        ],
    ),
    (
        "collator_create",
        &[(0, "locale", "string", false, false, true, false, false)],
    ),
    (
        "collator_get_attribute",
        &[(1, "attribute", "int", false, false, false, false, false)],
    ),
    (
        "collator_get_locale",
        &[(1, "type", "int", false, false, false, false, false)],
    ),
    (
        "collator_get_sort_key",
        &[(1, "string", "string", false, false, true, false, false)],
    ),
    (
        "collator_set_attribute",
        &[
            (1, "attribute", "int", false, false, false, false, false),
            (2, "value", "int", false, false, false, false, false),
        ],
    ),
    (
        "collator_set_strength",
        &[(1, "strength", "int", false, false, false, false, false)],
    ),
    (
        "collator_sort",
        &[
            (1, "array", "array", false, false, false, true, true),
            (2, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "collator_sort_with_sort_keys",
        &[(1, "array", "array", false, false, false, true, true)],
    ),
    (
        "com_event_sink",
        &[(
            2,
            "sink_interface",
            "array|string|null",
            false,
            true,
            true,
            true,
            false,
        )],
    ),
    (
        "com_get_active_object",
        &[
            (0, "prog_id", "string", false, false, true, false, false),
            (1, "codepage", "?int", false, true, false, false, false),
        ],
    ),
    (
        "com_load_typelib",
        &[
            (0, "typelib", "string", false, false, true, false, false),
            (
                1,
                "case_insensitive",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "com_message_pump",
        &[(
            0,
            "timeout_milliseconds",
            "int",
            false,
            false,
            false,
            false,
            false,
        )],
    ),
    (
        "com_print_typeinfo",
        &[
            (
                1,
                "dispatch_interface",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
            (2, "display_sink", "bool", false, false, false, false, false),
        ],
    ),
    (
        "constant",
        &[(0, "name", "string", false, false, true, false, false)],
    ),
    (
        "convert_uudecode",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "convert_uuencode",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "copy",
        &[
            (0, "from", "string", false, false, true, false, false),
            (1, "to", "string", false, false, true, false, false),
        ],
    ),
    (
        "cos",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "cosh",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "count",
        &[(1, "mode", "int", false, false, false, false, false)],
    ),
    (
        "count_chars",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "mode", "int", false, false, false, false, false),
        ],
    ),
    (
        "crc32",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "crypt",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "salt", "string", false, false, true, false, false),
        ],
    ),
    (
        "curl_escape",
        &[(1, "string", "string", false, false, true, false, false)],
    ),
    (
        "curl_file_create",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "mime_type", "?string", false, true, true, false, false),
            (
                2,
                "posted_filename",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "curl_getinfo",
        &[(1, "option", "?int", false, true, false, false, false)],
    ),
    (
        "curl_init",
        &[(0, "url", "?string", false, true, true, false, false)],
    ),
    (
        "curl_multi_select",
        &[(1, "timeout", "float", false, false, false, false, false)],
    ),
    (
        "curl_multi_setopt",
        &[(1, "option", "int", false, false, false, false, false)],
    ),
    (
        "curl_multi_strerror",
        &[(0, "error_code", "int", false, false, false, false, false)],
    ),
    (
        "curl_pause",
        &[(1, "flags", "int", false, false, false, false, false)],
    ),
    (
        "curl_setopt",
        &[(1, "option", "int", false, false, false, false, false)],
    ),
    (
        "curl_setopt_array",
        &[(1, "options", "array", false, false, false, true, false)],
    ),
    (
        "curl_share_init_persistent",
        &[(
            0,
            "share_options",
            "array",
            false,
            false,
            false,
            true,
            false,
        )],
    ),
    (
        "curl_share_setopt",
        &[(1, "option", "int", false, false, false, false, false)],
    ),
    (
        "curl_share_strerror",
        &[(0, "error_code", "int", false, false, false, false, false)],
    ),
    (
        "curl_strerror",
        &[(0, "error_code", "int", false, false, false, false, false)],
    ),
    (
        "curl_unescape",
        &[(1, "string", "string", false, false, true, false, false)],
    ),
    (
        "date",
        &[
            (0, "format", "string", false, false, true, false, false),
            (1, "timestamp", "?int", false, true, false, false, false),
        ],
    ),
    (
        "date_create",
        &[(0, "datetime", "string", false, false, true, false, false)],
    ),
    (
        "date_create_from_format",
        &[
            (0, "format", "string", false, false, true, false, false),
            (1, "datetime", "string", false, false, true, false, false),
        ],
    ),
    (
        "date_create_immutable",
        &[(0, "datetime", "string", false, false, true, false, false)],
    ),
    (
        "date_create_immutable_from_format",
        &[
            (0, "format", "string", false, false, true, false, false),
            (1, "datetime", "string", false, false, true, false, false),
        ],
    ),
    (
        "date_date_set",
        &[
            (1, "year", "int", false, false, false, false, false),
            (2, "month", "int", false, false, false, false, false),
            (3, "day", "int", false, false, false, false, false),
        ],
    ),
    (
        "date_default_timezone_set",
        &[(0, "timezoneId", "string", false, false, true, false, false)],
    ),
    (
        "date_diff",
        &[(2, "absolute", "bool", false, false, false, false, false)],
    ),
    (
        "date_format",
        &[(1, "format", "string", false, false, true, false, false)],
    ),
    (
        "date_interval_create_from_date_string",
        &[(0, "datetime", "string", false, false, true, false, false)],
    ),
    (
        "date_interval_format",
        &[(1, "format", "string", false, false, true, false, false)],
    ),
    (
        "date_isodate_set",
        &[
            (1, "year", "int", false, false, false, false, false),
            (2, "week", "int", false, false, false, false, false),
            (3, "dayOfWeek", "int", false, false, false, false, false),
        ],
    ),
    (
        "date_modify",
        &[(1, "modifier", "string", false, false, true, false, false)],
    ),
    (
        "date_parse",
        &[(0, "datetime", "string", false, false, true, false, false)],
    ),
    (
        "date_parse_from_format",
        &[
            (0, "format", "string", false, false, true, false, false),
            (1, "datetime", "string", false, false, true, false, false),
        ],
    ),
    (
        "date_sun_info",
        &[
            (0, "timestamp", "int", false, false, false, false, false),
            (1, "latitude", "float", false, false, false, false, false),
            (2, "longitude", "float", false, false, false, false, false),
        ],
    ),
    (
        "date_sunrise",
        &[
            (0, "timestamp", "int", false, false, false, false, false),
            (1, "returnFormat", "int", false, false, false, false, false),
            (2, "latitude", "?float", false, true, false, false, false),
            (3, "longitude", "?float", false, true, false, false, false),
            (4, "zenith", "?float", false, true, false, false, false),
            (5, "utcOffset", "?float", false, true, false, false, false),
        ],
    ),
    (
        "date_sunset",
        &[
            (0, "timestamp", "int", false, false, false, false, false),
            (1, "returnFormat", "int", false, false, false, false, false),
            (2, "latitude", "?float", false, true, false, false, false),
            (3, "longitude", "?float", false, true, false, false, false),
            (4, "zenith", "?float", false, true, false, false, false),
            (5, "utcOffset", "?float", false, true, false, false, false),
        ],
    ),
    (
        "date_time_set",
        &[
            (1, "hour", "int", false, false, false, false, false),
            (2, "minute", "int", false, false, false, false, false),
            (3, "second", "int", false, false, false, false, false),
            (4, "microsecond", "int", false, false, false, false, false),
        ],
    ),
    (
        "date_timestamp_set",
        &[(1, "timestamp", "int", false, false, false, false, false)],
    ),
    (
        "datefmt_create",
        &[
            (0, "locale", "?string", false, true, true, false, false),
            (1, "dateType", "int", false, false, false, false, false),
            (2, "timeType", "int", false, false, false, false, false),
            (5, "pattern", "?string", false, true, true, false, false),
        ],
    ),
    (
        "datefmt_format_object",
        &[(2, "locale", "?string", false, true, true, false, false)],
    ),
    (
        "datefmt_get_locale",
        &[(1, "type", "int", false, false, false, false, false)],
    ),
    (
        "datefmt_localtime",
        &[(1, "string", "string", false, false, true, false, false)],
    ),
    (
        "datefmt_parse",
        &[(1, "string", "string", false, false, true, false, false)],
    ),
    (
        "datefmt_set_lenient",
        &[(1, "lenient", "bool", false, false, false, false, false)],
    ),
    (
        "datefmt_set_pattern",
        &[(1, "pattern", "string", false, false, true, false, false)],
    ),
    (
        "dcgettext",
        &[
            (0, "domain", "string", false, false, true, false, false),
            (1, "message", "string", false, false, true, false, false),
            (2, "category", "int", false, false, false, false, false),
        ],
    ),
    (
        "dcngettext",
        &[
            (0, "domain", "string", false, false, true, false, false),
            (1, "singular", "string", false, false, true, false, false),
            (2, "plural", "string", false, false, true, false, false),
            (3, "count", "int", false, false, false, false, false),
            (4, "category", "int", false, false, false, false, false),
        ],
    ),
    (
        "debug_backtrace",
        &[
            (0, "options", "int", false, false, false, false, false),
            (1, "limit", "int", false, false, false, false, false),
        ],
    ),
    (
        "debug_print_backtrace",
        &[
            (0, "options", "int", false, false, false, false, false),
            (1, "limit", "int", false, false, false, false, false),
        ],
    ),
    (
        "decbin",
        &[(0, "num", "int", false, false, false, false, false)],
    ),
    (
        "dechex",
        &[(0, "num", "int", false, false, false, false, false)],
    ),
    (
        "decoct",
        &[(0, "num", "int", false, false, false, false, false)],
    ),
    (
        "define",
        &[
            (
                0,
                "constant_name",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                2,
                "case_insensitive",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "defined",
        &[(
            0,
            "constant_name",
            "string",
            false,
            false,
            true,
            false,
            false,
        )],
    ),
    (
        "deflate_add",
        &[
            (1, "data", "string", false, false, true, false, false),
            (2, "flush_mode", "int", false, false, false, false, false),
        ],
    ),
    (
        "deflate_init",
        &[(0, "encoding", "int", false, false, false, false, false)],
    ),
    (
        "deg2rad",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "dgettext",
        &[
            (0, "domain", "string", false, false, true, false, false),
            (1, "message", "string", false, false, true, false, false),
        ],
    ),
    (
        "die",
        &[(0, "status", "string|int", false, false, true, false, false)],
    ),
    (
        "dir",
        &[(0, "directory", "string", false, false, true, false, false)],
    ),
    (
        "dirname",
        &[
            (0, "path", "string", false, false, true, false, false),
            (1, "levels", "int", false, false, false, false, false),
        ],
    ),
    (
        "disk_free_space",
        &[(0, "directory", "string", false, false, true, false, false)],
    ),
    (
        "disk_total_space",
        &[(0, "directory", "string", false, false, true, false, false)],
    ),
    (
        "diskfreespace",
        &[(0, "directory", "string", false, false, true, false, false)],
    ),
    (
        "dl",
        &[(
            0,
            "extension_filename",
            "string",
            false,
            false,
            true,
            false,
            false,
        )],
    ),
    (
        "dngettext",
        &[
            (0, "domain", "string", false, false, true, false, false),
            (1, "singular", "string", false, false, true, false, false),
            (2, "plural", "string", false, false, true, false, false),
            (3, "count", "int", false, false, false, false, false),
        ],
    ),
    (
        "dns_check_record",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (1, "type", "string", false, false, true, false, false),
        ],
    ),
    (
        "dns_get_mx",
        &[(0, "hostname", "string", false, false, true, false, false)],
    ),
    (
        "dns_get_record",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (1, "type", "int", false, false, false, false, false),
            (4, "raw", "bool", false, false, false, false, false),
        ],
    ),
    (
        "easter_date",
        &[
            (0, "year", "?int", false, true, false, false, false),
            (1, "mode", "int", false, false, false, false, false),
        ],
    ),
    (
        "easter_days",
        &[
            (0, "year", "?int", false, true, false, false, false),
            (1, "mode", "int", false, false, false, false, false),
        ],
    ),
    (
        "enchant_broker_dict_exists",
        &[(1, "tag", "string", false, false, true, false, false)],
    ),
    (
        "enchant_broker_get_dict_path",
        &[(1, "type", "int", false, false, false, false, false)],
    ),
    (
        "enchant_broker_request_dict",
        &[(1, "tag", "string", false, false, true, false, false)],
    ),
    (
        "enchant_broker_request_pwl_dict",
        &[(1, "filename", "string", false, false, true, false, false)],
    ),
    (
        "enchant_broker_set_dict_path",
        &[
            (1, "type", "int", false, false, false, false, false),
            (2, "path", "string", false, false, true, false, false),
        ],
    ),
    (
        "enchant_broker_set_ordering",
        &[
            (1, "tag", "string", false, false, true, false, false),
            (2, "ordering", "string", false, false, true, false, false),
        ],
    ),
    (
        "enchant_dict_add",
        &[(1, "word", "string", false, false, true, false, false)],
    ),
    (
        "enchant_dict_add_to_personal",
        &[(1, "word", "string", false, false, true, false, false)],
    ),
    (
        "enchant_dict_add_to_session",
        &[(1, "word", "string", false, false, true, false, false)],
    ),
    (
        "enchant_dict_check",
        &[(1, "word", "string", false, false, true, false, false)],
    ),
    (
        "enchant_dict_is_added",
        &[(1, "word", "string", false, false, true, false, false)],
    ),
    (
        "enchant_dict_is_in_session",
        &[(1, "word", "string", false, false, true, false, false)],
    ),
    (
        "enchant_dict_quick_check",
        &[(1, "word", "string", false, false, true, false, false)],
    ),
    (
        "enchant_dict_remove",
        &[(1, "word", "string", false, false, true, false, false)],
    ),
    (
        "enchant_dict_remove_from_session",
        &[(1, "word", "string", false, false, true, false, false)],
    ),
    (
        "enchant_dict_store_replacement",
        &[
            (1, "misspelled", "string", false, false, true, false, false),
            (2, "correct", "string", false, false, true, false, false),
        ],
    ),
    (
        "enchant_dict_suggest",
        &[(1, "word", "string", false, false, true, false, false)],
    ),
    (
        "enum_exists",
        &[
            (0, "enum", "string", false, false, true, false, false),
            (1, "autoload", "bool", false, false, false, false, false),
        ],
    ),
    (
        "error_log",
        &[
            (0, "message", "string", false, false, true, false, false),
            (1, "message_type", "int", false, false, false, false, false),
            (2, "destination", "?string", false, true, true, false, false),
            (
                3,
                "additional_headers",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "error_reporting",
        &[(0, "error_level", "?int", false, true, false, false, false)],
    ),
    (
        "escapeshellarg",
        &[(0, "arg", "string", false, false, true, false, false)],
    ),
    (
        "escapeshellcmd",
        &[(0, "command", "string", false, false, true, false, false)],
    ),
    (
        "exec",
        &[(0, "command", "string", false, false, true, false, false)],
    ),
    (
        "exif_imagetype",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "exif_read_data",
        &[
            (
                1,
                "required_sections",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
            (2, "as_arrays", "bool", false, false, false, false, false),
            (
                3,
                "read_thumbnail",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "exif_tagname",
        &[(0, "index", "int", false, false, false, false, false)],
    ),
    (
        "exit",
        &[(0, "status", "string|int", false, false, true, false, false)],
    ),
    (
        "exp",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "explode",
        &[
            (0, "separator", "string", false, false, true, false, false),
            (1, "string", "string", false, false, true, false, false),
            (2, "limit", "int", false, false, false, false, false),
        ],
    ),
    (
        "expm1",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "extension_loaded",
        &[(0, "extension", "string", false, false, true, false, false)],
    ),
    (
        "extract",
        &[
            (0, "array", "array", false, false, false, true, true),
            (1, "flags", "int", false, false, false, false, false),
            (2, "prefix", "string", false, false, true, false, false),
        ],
    ),
    (
        "fdiv",
        &[
            (0, "num1", "float", false, false, false, false, false),
            (1, "num2", "float", false, false, false, false, false),
        ],
    ),
    (
        "fgetcsv",
        &[
            (1, "length", "?int", false, true, false, false, false),
            (2, "separator", "string", false, false, true, false, false),
            (3, "enclosure", "string", false, false, true, false, false),
        ],
    ),
    (
        "fgets",
        &[(1, "length", "?int", false, true, false, false, false)],
    ),
    (
        "file",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "file_exists",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "file_get_contents",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (
                1,
                "use_include_path",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
            (3, "offset", "int", false, false, false, false, false),
            (4, "length", "?int", false, true, false, false, false),
        ],
    ),
    (
        "file_put_contents",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (2, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "fileatime",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "filectime",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "filegroup",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "fileinode",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "filemtime",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "fileowner",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "fileperms",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "filesize",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "filetype",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "filter_has_var",
        &[
            (0, "input_type", "int", false, false, false, false, false),
            (1, "var_name", "string", false, false, true, false, false),
        ],
    ),
    (
        "filter_id",
        &[(0, "name", "string", false, false, true, false, false)],
    ),
    (
        "filter_input",
        &[
            (0, "type", "int", false, false, false, false, false),
            (1, "var_name", "string", false, false, true, false, false),
            (2, "filter", "int", false, false, false, false, false),
            (3, "options", "array|int", false, false, false, true, false),
        ],
    ),
    (
        "filter_input_array",
        &[
            (0, "type", "int", false, false, false, false, false),
            (1, "options", "array|int", false, false, false, true, false),
            (2, "add_empty", "bool", false, false, false, false, false),
        ],
    ),
    (
        "filter_var",
        &[
            (1, "filter", "int", false, false, false, false, false),
            (2, "options", "array|int", false, false, false, true, false),
        ],
    ),
    (
        "filter_var_array",
        &[
            (0, "array", "array", false, false, false, true, false),
            (1, "options", "array|int", false, false, false, true, false),
            (2, "add_empty", "bool", false, false, false, false, false),
        ],
    ),
    (
        "finfo_buffer",
        &[
            (1, "string", "string", false, false, true, false, false),
            (2, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "finfo_file",
        &[
            (1, "filename", "string", false, false, true, false, false),
            (2, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "finfo_open",
        &[
            (0, "flags", "int", false, false, false, false, false),
            (
                1,
                "magic_database",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "finfo_set_flags",
        &[(1, "flags", "int", false, false, false, false, false)],
    ),
    (
        "flock",
        &[(1, "operation", "int", false, false, false, false, false)],
    ),
    (
        "floor",
        &[(0, "num", "int|float", false, false, false, false, false)],
    ),
    (
        "fmod",
        &[
            (0, "num1", "float", false, false, false, false, false),
            (1, "num2", "float", false, false, false, false, false),
        ],
    ),
    (
        "fnmatch",
        &[
            (0, "pattern", "string", false, false, true, false, false),
            (1, "filename", "string", false, false, true, false, false),
            (2, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "fopen",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "mode", "string", false, false, true, false, false),
            (
                2,
                "use_include_path",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "forward_static_call_array",
        &[(1, "args", "array", false, false, false, true, false)],
    ),
    (
        "fpow",
        &[
            (0, "num", "float", false, false, false, false, false),
            (1, "exponent", "float", false, false, false, false, false),
        ],
    ),
    (
        "fprintf",
        &[(1, "format", "string", false, false, true, false, false)],
    ),
    (
        "fputcsv",
        &[
            (1, "fields", "array", false, false, false, true, false),
            (2, "separator", "string", false, false, true, false, false),
            (3, "enclosure", "string", false, false, true, false, false),
        ],
    ),
    (
        "fputs",
        &[
            (1, "data", "string", false, false, true, false, false),
            (2, "length", "?int", false, true, false, false, false),
        ],
    ),
    (
        "fread",
        &[(1, "length", "int", false, false, false, false, false)],
    ),
    (
        "frenchtojd",
        &[
            (0, "month", "int", false, false, false, false, false),
            (1, "day", "int", false, false, false, false, false),
            (2, "year", "int", false, false, false, false, false),
        ],
    ),
    (
        "fscanf",
        &[(1, "format", "string", false, false, true, false, false)],
    ),
    (
        "fseek",
        &[
            (1, "offset", "int", false, false, false, false, false),
            (2, "whence", "int", false, false, false, false, false),
        ],
    ),
    (
        "fsockopen",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (1, "port", "int", false, false, false, false, false),
            (4, "timeout", "?float", false, true, false, false, false),
        ],
    ),
    (
        "ftok",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "project_id", "string", false, false, true, false, false),
        ],
    ),
    (
        "ftruncate",
        &[(1, "size", "int", false, false, false, false, false)],
    ),
    (
        "func_get_arg",
        &[(0, "position", "int", false, false, false, false, false)],
    ),
    (
        "function_exists",
        &[(0, "function", "string", false, false, true, false, false)],
    ),
    (
        "fwrite",
        &[
            (1, "data", "string", false, false, true, false, false),
            (2, "length", "?int", false, true, false, false, false),
        ],
    ),
    (
        "get_browser",
        &[
            (0, "user_agent", "?string", false, true, true, false, false),
            (1, "return_array", "bool", false, false, false, false, false),
        ],
    ),
    (
        "get_cfg_var",
        &[(0, "option", "string", false, false, true, false, false)],
    ),
    (
        "get_class_vars",
        &[(0, "class", "string", false, false, true, false, false)],
    ),
    (
        "get_defined_constants",
        &[(0, "categorize", "bool", false, false, false, false, false)],
    ),
    (
        "get_defined_functions",
        &[(
            0,
            "exclude_disabled",
            "bool",
            false,
            false,
            false,
            false,
            false,
        )],
    ),
    (
        "get_extension_funcs",
        &[(0, "extension", "string", false, false, true, false, false)],
    ),
    (
        "get_headers",
        &[
            (0, "url", "string", false, false, true, false, false),
            (1, "associative", "bool", false, false, false, false, false),
        ],
    ),
    (
        "get_html_translation_table",
        &[
            (0, "table", "int", false, false, false, false, false),
            (1, "flags", "int", false, false, false, false, false),
            (2, "encoding", "string", false, false, true, false, false),
        ],
    ),
    (
        "get_loaded_extensions",
        &[(
            0,
            "zend_extensions",
            "bool",
            false,
            false,
            false,
            false,
            false,
        )],
    ),
    (
        "get_meta_tags",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (
                1,
                "use_include_path",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "get_resources",
        &[(0, "type", "?string", false, true, true, false, false)],
    ),
    (
        "getdate",
        &[(0, "timestamp", "?int", false, true, false, false, false)],
    ),
    (
        "getenv",
        &[
            (0, "name", "?string", false, true, true, false, false),
            (1, "local_only", "bool", false, false, false, false, false),
        ],
    ),
    (
        "gethostbyaddr",
        &[(0, "ip", "string", false, false, true, false, false)],
    ),
    (
        "gethostbyname",
        &[(0, "hostname", "string", false, false, true, false, false)],
    ),
    (
        "gethostbynamel",
        &[(0, "hostname", "string", false, false, true, false, false)],
    ),
    (
        "getimagesize",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "getimagesizefromstring",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "getmxrr",
        &[(0, "hostname", "string", false, false, true, false, false)],
    ),
    (
        "getopt",
        &[
            (
                0,
                "short_options",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (1, "long_options", "array", false, false, false, true, false),
        ],
    ),
    (
        "getprotobyname",
        &[(0, "protocol", "string", false, false, true, false, false)],
    ),
    (
        "getprotobynumber",
        &[(0, "protocol", "int", false, false, false, false, false)],
    ),
    (
        "getrusage",
        &[(0, "mode", "int", false, false, false, false, false)],
    ),
    (
        "getservbyname",
        &[
            (0, "service", "string", false, false, true, false, false),
            (1, "protocol", "string", false, false, true, false, false),
        ],
    ),
    (
        "getservbyport",
        &[
            (0, "port", "int", false, false, false, false, false),
            (1, "protocol", "string", false, false, true, false, false),
        ],
    ),
    (
        "gettext",
        &[(0, "message", "string", false, false, true, false, false)],
    ),
    (
        "gettimeofday",
        &[(0, "as_float", "bool", false, false, false, false, false)],
    ),
    (
        "glob",
        &[
            (0, "pattern", "string", false, false, true, false, false),
            (1, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "gmdate",
        &[
            (0, "format", "string", false, false, true, false, false),
            (1, "timestamp", "?int", false, true, false, false, false),
        ],
    ),
    (
        "gmmktime",
        &[
            (0, "hour", "int", false, false, false, false, false),
            (1, "minute", "?int", false, true, false, false, false),
            (2, "second", "?int", false, true, false, false, false),
            (3, "month", "?int", false, true, false, false, false),
            (4, "day", "?int", false, true, false, false, false),
            (5, "year", "?int", false, true, false, false, false),
        ],
    ),
    (
        "gmp_binomial",
        &[(1, "k", "int", false, false, false, false, false)],
    ),
    (
        "gmp_clrbit",
        &[(1, "index", "int", false, false, false, false, false)],
    ),
    (
        "gmp_div",
        &[(2, "rounding_mode", "int", false, false, false, false, false)],
    ),
    (
        "gmp_div_q",
        &[(2, "rounding_mode", "int", false, false, false, false, false)],
    ),
    (
        "gmp_div_qr",
        &[(2, "rounding_mode", "int", false, false, false, false, false)],
    ),
    (
        "gmp_div_r",
        &[(2, "rounding_mode", "int", false, false, false, false, false)],
    ),
    (
        "gmp_export",
        &[
            (1, "word_size", "int", false, false, false, false, false),
            (2, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "gmp_import",
        &[
            (0, "data", "string", false, false, true, false, false),
            (1, "word_size", "int", false, false, false, false, false),
            (2, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "gmp_init",
        &[
            (0, "num", "string|int", false, false, true, false, false),
            (1, "base", "int", false, false, false, false, false),
        ],
    ),
    (
        "gmp_pow",
        &[(1, "exponent", "int", false, false, false, false, false)],
    ),
    (
        "gmp_prob_prime",
        &[(1, "repetitions", "int", false, false, false, false, false)],
    ),
    (
        "gmp_random_bits",
        &[(0, "bits", "int", false, false, false, false, false)],
    ),
    (
        "gmp_root",
        &[(1, "nth", "int", false, false, false, false, false)],
    ),
    (
        "gmp_rootrem",
        &[(1, "nth", "int", false, false, false, false, false)],
    ),
    (
        "gmp_scan0",
        &[(1, "start", "int", false, false, false, false, false)],
    ),
    (
        "gmp_scan1",
        &[(1, "start", "int", false, false, false, false, false)],
    ),
    (
        "gmp_setbit",
        &[
            (1, "index", "int", false, false, false, false, false),
            (2, "value", "bool", false, false, false, false, false),
        ],
    ),
    (
        "gmp_strval",
        &[(1, "base", "int", false, false, false, false, false)],
    ),
    (
        "gmp_testbit",
        &[(1, "index", "int", false, false, false, false, false)],
    ),
    (
        "gmstrftime",
        &[
            (0, "format", "string", false, false, true, false, false),
            (1, "timestamp", "?int", false, true, false, false, false),
        ],
    ),
    (
        "grapheme_extract",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "size", "int", false, false, false, false, false),
            (2, "type", "int", false, false, false, false, false),
            (3, "offset", "int", false, false, false, false, false),
        ],
    ),
    (
        "grapheme_levenshtein",
        &[
            (0, "string1", "string", false, false, true, false, false),
            (1, "string2", "string", false, false, true, false, false),
            (
                2,
                "insertion_cost",
                "int",
                false,
                false,
                false,
                false,
                false,
            ),
            (
                3,
                "replacement_cost",
                "int",
                false,
                false,
                false,
                false,
                false,
            ),
            (4, "deletion_cost", "int", false, false, false, false, false),
            (5, "locale", "string", false, false, true, false, false),
        ],
    ),
    (
        "grapheme_str_split",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "length", "int", false, false, false, false, false),
        ],
    ),
    (
        "grapheme_stripos",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (2, "offset", "int", false, false, false, false, false),
            (3, "locale", "string", false, false, true, false, false),
        ],
    ),
    (
        "grapheme_stristr",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (2, "beforeNeedle", "bool", false, false, false, false, false),
            (3, "locale", "string", false, false, true, false, false),
        ],
    ),
    (
        "grapheme_strlen",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "grapheme_strpos",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (2, "offset", "int", false, false, false, false, false),
            (3, "locale", "string", false, false, true, false, false),
        ],
    ),
    (
        "grapheme_strripos",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (2, "offset", "int", false, false, false, false, false),
            (3, "locale", "string", false, false, true, false, false),
        ],
    ),
    (
        "grapheme_strrpos",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (2, "offset", "int", false, false, false, false, false),
            (3, "locale", "string", false, false, true, false, false),
        ],
    ),
    (
        "grapheme_strstr",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (2, "beforeNeedle", "bool", false, false, false, false, false),
            (3, "locale", "string", false, false, true, false, false),
        ],
    ),
    (
        "grapheme_substr",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "offset", "int", false, false, false, false, false),
            (2, "length", "?int", false, true, false, false, false),
            (3, "locale", "string", false, false, true, false, false),
        ],
    ),
    (
        "gregoriantojd",
        &[
            (0, "month", "int", false, false, false, false, false),
            (1, "day", "int", false, false, false, false, false),
            (2, "year", "int", false, false, false, false, false),
        ],
    ),
    (
        "gzcompress",
        &[
            (0, "data", "string", false, false, true, false, false),
            (1, "level", "int", false, false, false, false, false),
            (2, "encoding", "int", false, false, false, false, false),
        ],
    ),
    (
        "gzdecode",
        &[
            (0, "data", "string", false, false, true, false, false),
            (1, "max_length", "int", false, false, false, false, false),
        ],
    ),
    (
        "gzdeflate",
        &[
            (0, "data", "string", false, false, true, false, false),
            (1, "level", "int", false, false, false, false, false),
            (2, "encoding", "int", false, false, false, false, false),
        ],
    ),
    (
        "gzencode",
        &[
            (0, "data", "string", false, false, true, false, false),
            (1, "level", "int", false, false, false, false, false),
            (2, "encoding", "int", false, false, false, false, false),
        ],
    ),
    (
        "gzfile",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (
                1,
                "use_include_path",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "gzgets",
        &[(1, "length", "?int", false, true, false, false, false)],
    ),
    (
        "gzinflate",
        &[
            (0, "data", "string", false, false, true, false, false),
            (1, "max_length", "int", false, false, false, false, false),
        ],
    ),
    (
        "gzopen",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "mode", "string", false, false, true, false, false),
            (
                2,
                "use_include_path",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "gzputs",
        &[
            (1, "data", "string", false, false, true, false, false),
            (2, "length", "?int", false, true, false, false, false),
        ],
    ),
    (
        "gzread",
        &[(1, "length", "int", false, false, false, false, false)],
    ),
    (
        "gzseek",
        &[
            (1, "offset", "int", false, false, false, false, false),
            (2, "whence", "int", false, false, false, false, false),
        ],
    ),
    (
        "gzuncompress",
        &[
            (0, "data", "string", false, false, true, false, false),
            (1, "max_length", "int", false, false, false, false, false),
        ],
    ),
    (
        "gzwrite",
        &[
            (1, "data", "string", false, false, true, false, false),
            (2, "length", "?int", false, true, false, false, false),
        ],
    ),
    (
        "hash",
        &[
            (0, "algo", "string", false, false, true, false, false),
            (1, "data", "string", false, false, true, false, false),
            (2, "binary", "bool", false, false, false, false, false),
            (3, "options", "array", false, false, false, true, false),
        ],
    ),
    (
        "hash_equals",
        &[
            (
                0,
                "known_string",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (1, "user_string", "string", false, false, true, false, false),
        ],
    ),
    (
        "hash_file",
        &[
            (0, "algo", "string", false, false, true, false, false),
            (1, "filename", "string", false, false, true, false, false),
            (2, "binary", "bool", false, false, false, false, false),
            (3, "options", "array", false, false, false, true, false),
        ],
    ),
    (
        "hash_final",
        &[(1, "binary", "bool", false, false, false, false, false)],
    ),
    (
        "hash_hkdf",
        &[
            (0, "algo", "string", false, false, true, false, false),
            (1, "key", "string", false, false, true, false, false),
            (2, "length", "int", false, false, false, false, false),
            (3, "info", "string", false, false, true, false, false),
            (4, "salt", "string", false, false, true, false, false),
        ],
    ),
    (
        "hash_hmac",
        &[
            (0, "algo", "string", false, false, true, false, false),
            (1, "data", "string", false, false, true, false, false),
            (2, "key", "string", false, false, true, false, false),
            (3, "binary", "bool", false, false, false, false, false),
        ],
    ),
    (
        "hash_hmac_file",
        &[
            (0, "algo", "string", false, false, true, false, false),
            (1, "filename", "string", false, false, true, false, false),
            (2, "key", "string", false, false, true, false, false),
            (3, "binary", "bool", false, false, false, false, false),
        ],
    ),
    (
        "hash_init",
        &[
            (0, "algo", "string", false, false, true, false, false),
            (1, "flags", "int", false, false, false, false, false),
            (2, "key", "string", false, false, true, false, false),
            (3, "options", "array", false, false, false, true, false),
        ],
    ),
    (
        "hash_pbkdf2",
        &[
            (0, "algo", "string", false, false, true, false, false),
            (1, "password", "string", false, false, true, false, false),
            (2, "salt", "string", false, false, true, false, false),
            (3, "iterations", "int", false, false, false, false, false),
            (4, "length", "int", false, false, false, false, false),
            (5, "binary", "bool", false, false, false, false, false),
            (6, "options", "array", false, false, false, true, false),
        ],
    ),
    (
        "hash_update",
        &[(1, "data", "string", false, false, true, false, false)],
    ),
    (
        "hash_update_file",
        &[(1, "filename", "string", false, false, true, false, false)],
    ),
    (
        "hash_update_stream",
        &[(2, "length", "int", false, false, false, false, false)],
    ),
    (
        "header",
        &[
            (0, "header", "string", false, false, true, false, false),
            (1, "replace", "bool", false, false, false, false, false),
            (2, "response_code", "int", false, false, false, false, false),
        ],
    ),
    (
        "header_remove",
        &[(0, "name", "?string", false, true, true, false, false)],
    ),
    (
        "hebrev",
        &[
            (0, "string", "string", false, false, true, false, false),
            (
                1,
                "max_chars_per_line",
                "int",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "hex2bin",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "hexdec",
        &[(0, "hex_string", "string", false, false, true, false, false)],
    ),
    (
        "highlight_file",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "return", "bool", false, false, false, false, false),
        ],
    ),
    (
        "highlight_string",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "return", "bool", false, false, false, false, false),
        ],
    ),
    (
        "hrtime",
        &[(0, "as_number", "bool", false, false, false, false, false)],
    ),
    (
        "html_entity_decode",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "flags", "int", false, false, false, false, false),
            (2, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "htmlentities",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "flags", "int", false, false, false, false, false),
            (2, "encoding", "?string", false, true, true, false, false),
            (
                3,
                "double_encode",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "htmlspecialchars",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "flags", "int", false, false, false, false, false),
            (2, "encoding", "?string", false, true, true, false, false),
            (
                3,
                "double_encode",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "htmlspecialchars_decode",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "http_build_query",
        &[
            (
                1,
                "numeric_prefix",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                2,
                "arg_separator",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
            (3, "encoding_type", "int", false, false, false, false, false),
        ],
    ),
    (
        "http_response_code",
        &[(0, "response_code", "int", false, false, false, false, false)],
    ),
    (
        "hypot",
        &[
            (0, "x", "float", false, false, false, false, false),
            (1, "y", "float", false, false, false, false, false),
        ],
    ),
    (
        "iconv",
        &[
            (
                0,
                "from_encoding",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (1, "to_encoding", "string", false, false, true, false, false),
            (2, "string", "string", false, false, true, false, false),
        ],
    ),
    (
        "iconv_get_encoding",
        &[(0, "type", "string", false, false, true, false, false)],
    ),
    (
        "iconv_mime_decode",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "mode", "int", false, false, false, false, false),
            (2, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "iconv_mime_decode_headers",
        &[
            (0, "headers", "string", false, false, true, false, false),
            (1, "mode", "int", false, false, false, false, false),
            (2, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "iconv_mime_encode",
        &[
            (0, "field_name", "string", false, false, true, false, false),
            (1, "field_value", "string", false, false, true, false, false),
            (2, "options", "array", false, false, false, true, false),
        ],
    ),
    (
        "iconv_set_encoding",
        &[
            (0, "type", "string", false, false, true, false, false),
            (1, "encoding", "string", false, false, true, false, false),
        ],
    ),
    (
        "iconv_strlen",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "iconv_strpos",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (2, "offset", "int", false, false, false, false, false),
            (3, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "iconv_strrpos",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (2, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "iconv_substr",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "offset", "int", false, false, false, false, false),
            (2, "length", "?int", false, true, false, false, false),
            (3, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "idate",
        &[
            (0, "format", "string", false, false, true, false, false),
            (1, "timestamp", "?int", false, true, false, false, false),
        ],
    ),
    (
        "idn_to_ascii",
        &[
            (0, "domain", "string", false, false, true, false, false),
            (1, "flags", "int", false, false, false, false, false),
            (2, "variant", "int", false, false, false, false, false),
        ],
    ),
    (
        "idn_to_utf8",
        &[
            (0, "domain", "string", false, false, true, false, false),
            (1, "flags", "int", false, false, false, false, false),
            (2, "variant", "int", false, false, false, false, false),
        ],
    ),
    (
        "ignore_user_abort",
        &[(0, "enable", "?bool", false, true, false, false, false)],
    ),
    (
        "image_type_to_extension",
        &[
            (0, "image_type", "int", false, false, false, false, false),
            (1, "include_dot", "bool", false, false, false, false, false),
        ],
    ),
    (
        "image_type_to_mime_type",
        &[(0, "image_type", "int", false, false, false, false, false)],
    ),
    (
        "imageaffine",
        &[
            (1, "affine", "array", false, false, false, true, false),
            (2, "clip", "?array", false, true, false, true, false),
        ],
    ),
    (
        "imageaffinematrixconcat",
        &[
            (0, "matrix1", "array", false, false, false, true, false),
            (1, "matrix2", "array", false, false, false, true, false),
        ],
    ),
    (
        "imageaffinematrixget",
        &[(0, "type", "int", false, false, false, false, false)],
    ),
    (
        "imagealphablending",
        &[(1, "enable", "bool", false, false, false, false, false)],
    ),
    (
        "imageantialias",
        &[(1, "enable", "bool", false, false, false, false, false)],
    ),
    (
        "imagearc",
        &[
            (1, "center_x", "int", false, false, false, false, false),
            (2, "center_y", "int", false, false, false, false, false),
            (3, "width", "int", false, false, false, false, false),
            (4, "height", "int", false, false, false, false, false),
            (5, "start_angle", "int", false, false, false, false, false),
            (6, "end_angle", "int", false, false, false, false, false),
            (7, "color", "int", false, false, false, false, false),
        ],
    ),
    (
        "imageavif",
        &[
            (2, "quality", "int", false, false, false, false, false),
            (3, "speed", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagebmp",
        &[(2, "compressed", "bool", false, false, false, false, false)],
    ),
    (
        "imagechar",
        &[
            (2, "x", "int", false, false, false, false, false),
            (3, "y", "int", false, false, false, false, false),
            (4, "char", "string", false, false, true, false, false),
            (5, "color", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecharup",
        &[
            (2, "x", "int", false, false, false, false, false),
            (3, "y", "int", false, false, false, false, false),
            (4, "char", "string", false, false, true, false, false),
            (5, "color", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecolorallocate",
        &[
            (1, "red", "int", false, false, false, false, false),
            (2, "green", "int", false, false, false, false, false),
            (3, "blue", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecolorallocatealpha",
        &[
            (1, "red", "int", false, false, false, false, false),
            (2, "green", "int", false, false, false, false, false),
            (3, "blue", "int", false, false, false, false, false),
            (4, "alpha", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecolorat",
        &[
            (1, "x", "int", false, false, false, false, false),
            (2, "y", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecolorclosest",
        &[
            (1, "red", "int", false, false, false, false, false),
            (2, "green", "int", false, false, false, false, false),
            (3, "blue", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecolorclosestalpha",
        &[
            (1, "red", "int", false, false, false, false, false),
            (2, "green", "int", false, false, false, false, false),
            (3, "blue", "int", false, false, false, false, false),
            (4, "alpha", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecolorclosesthwb",
        &[
            (1, "red", "int", false, false, false, false, false),
            (2, "green", "int", false, false, false, false, false),
            (3, "blue", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecolordeallocate",
        &[(1, "color", "int", false, false, false, false, false)],
    ),
    (
        "imagecolorexact",
        &[
            (1, "red", "int", false, false, false, false, false),
            (2, "green", "int", false, false, false, false, false),
            (3, "blue", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecolorexactalpha",
        &[
            (1, "red", "int", false, false, false, false, false),
            (2, "green", "int", false, false, false, false, false),
            (3, "blue", "int", false, false, false, false, false),
            (4, "alpha", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecolorresolve",
        &[
            (1, "red", "int", false, false, false, false, false),
            (2, "green", "int", false, false, false, false, false),
            (3, "blue", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecolorresolvealpha",
        &[
            (1, "red", "int", false, false, false, false, false),
            (2, "green", "int", false, false, false, false, false),
            (3, "blue", "int", false, false, false, false, false),
            (4, "alpha", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecolorset",
        &[
            (1, "color", "int", false, false, false, false, false),
            (2, "red", "int", false, false, false, false, false),
            (3, "green", "int", false, false, false, false, false),
            (4, "blue", "int", false, false, false, false, false),
            (5, "alpha", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecolorsforindex",
        &[(1, "color", "int", false, false, false, false, false)],
    ),
    (
        "imagecolortransparent",
        &[(1, "color", "?int", false, true, false, false, false)],
    ),
    (
        "imageconvolution",
        &[
            (1, "matrix", "array", false, false, false, true, false),
            (2, "divisor", "float", false, false, false, false, false),
            (3, "offset", "float", false, false, false, false, false),
        ],
    ),
    (
        "imagecopy",
        &[
            (2, "dst_x", "int", false, false, false, false, false),
            (3, "dst_y", "int", false, false, false, false, false),
            (4, "src_x", "int", false, false, false, false, false),
            (5, "src_y", "int", false, false, false, false, false),
            (6, "src_width", "int", false, false, false, false, false),
            (7, "src_height", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecopymerge",
        &[
            (2, "dst_x", "int", false, false, false, false, false),
            (3, "dst_y", "int", false, false, false, false, false),
            (4, "src_x", "int", false, false, false, false, false),
            (5, "src_y", "int", false, false, false, false, false),
            (6, "src_width", "int", false, false, false, false, false),
            (7, "src_height", "int", false, false, false, false, false),
            (8, "pct", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecopymergegray",
        &[
            (2, "dst_x", "int", false, false, false, false, false),
            (3, "dst_y", "int", false, false, false, false, false),
            (4, "src_x", "int", false, false, false, false, false),
            (5, "src_y", "int", false, false, false, false, false),
            (6, "src_width", "int", false, false, false, false, false),
            (7, "src_height", "int", false, false, false, false, false),
            (8, "pct", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecopyresampled",
        &[
            (2, "dst_x", "int", false, false, false, false, false),
            (3, "dst_y", "int", false, false, false, false, false),
            (4, "src_x", "int", false, false, false, false, false),
            (5, "src_y", "int", false, false, false, false, false),
            (6, "dst_width", "int", false, false, false, false, false),
            (7, "dst_height", "int", false, false, false, false, false),
            (8, "src_width", "int", false, false, false, false, false),
            (9, "src_height", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecopyresized",
        &[
            (2, "dst_x", "int", false, false, false, false, false),
            (3, "dst_y", "int", false, false, false, false, false),
            (4, "src_x", "int", false, false, false, false, false),
            (5, "src_y", "int", false, false, false, false, false),
            (6, "dst_width", "int", false, false, false, false, false),
            (7, "dst_height", "int", false, false, false, false, false),
            (8, "src_width", "int", false, false, false, false, false),
            (9, "src_height", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecreate",
        &[
            (0, "width", "int", false, false, false, false, false),
            (1, "height", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecreatefromavif",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "imagecreatefrombmp",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "imagecreatefromgd",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "imagecreatefromgd2",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "imagecreatefromgd2part",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "x", "int", false, false, false, false, false),
            (2, "y", "int", false, false, false, false, false),
            (3, "width", "int", false, false, false, false, false),
            (4, "height", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecreatefromgif",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "imagecreatefromjpeg",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "imagecreatefrompng",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "imagecreatefromstring",
        &[(0, "data", "string", false, false, true, false, false)],
    ),
    (
        "imagecreatefromtga",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "imagecreatefromwbmp",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "imagecreatefromwebp",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "imagecreatefromxbm",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "imagecreatefromxpm",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "imagecreatetruecolor",
        &[
            (0, "width", "int", false, false, false, false, false),
            (1, "height", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagecrop",
        &[(1, "rectangle", "array", false, false, false, true, false)],
    ),
    (
        "imagecropauto",
        &[
            (1, "mode", "int", false, false, false, false, false),
            (2, "threshold", "float", false, false, false, false, false),
            (3, "color", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagedashedline",
        &[
            (1, "x1", "int", false, false, false, false, false),
            (2, "y1", "int", false, false, false, false, false),
            (3, "x2", "int", false, false, false, false, false),
            (4, "y2", "int", false, false, false, false, false),
            (5, "color", "int", false, false, false, false, false),
        ],
    ),
    (
        "imageellipse",
        &[
            (1, "center_x", "int", false, false, false, false, false),
            (2, "center_y", "int", false, false, false, false, false),
            (3, "width", "int", false, false, false, false, false),
            (4, "height", "int", false, false, false, false, false),
            (5, "color", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagefill",
        &[
            (1, "x", "int", false, false, false, false, false),
            (2, "y", "int", false, false, false, false, false),
            (3, "color", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagefilledarc",
        &[
            (1, "center_x", "int", false, false, false, false, false),
            (2, "center_y", "int", false, false, false, false, false),
            (3, "width", "int", false, false, false, false, false),
            (4, "height", "int", false, false, false, false, false),
            (5, "start_angle", "int", false, false, false, false, false),
            (6, "end_angle", "int", false, false, false, false, false),
            (7, "color", "int", false, false, false, false, false),
            (8, "style", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagefilledellipse",
        &[
            (1, "center_x", "int", false, false, false, false, false),
            (2, "center_y", "int", false, false, false, false, false),
            (3, "width", "int", false, false, false, false, false),
            (4, "height", "int", false, false, false, false, false),
            (5, "color", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagefilledpolygon",
        &[
            (1, "points", "array", false, false, false, true, false),
            (
                2,
                "num_points_or_color",
                "int",
                false,
                false,
                false,
                false,
                false,
            ),
            (3, "color", "?int", false, true, false, false, false),
        ],
    ),
    (
        "imagefilledrectangle",
        &[
            (1, "x1", "int", false, false, false, false, false),
            (2, "y1", "int", false, false, false, false, false),
            (3, "x2", "int", false, false, false, false, false),
            (4, "y2", "int", false, false, false, false, false),
            (5, "color", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagefilltoborder",
        &[
            (1, "x", "int", false, false, false, false, false),
            (2, "y", "int", false, false, false, false, false),
            (3, "border_color", "int", false, false, false, false, false),
            (4, "color", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagefilter",
        &[(1, "filter", "int", false, false, false, false, false)],
    ),
    (
        "imageflip",
        &[(1, "mode", "int", false, false, false, false, false)],
    ),
    (
        "imageftbbox",
        &[
            (0, "size", "float", false, false, false, false, false),
            (1, "angle", "float", false, false, false, false, false),
            (
                2,
                "font_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (3, "string", "string", false, false, true, false, false),
            (4, "options", "array", false, false, false, true, false),
        ],
    ),
    (
        "imagefttext",
        &[
            (1, "size", "float", false, false, false, false, false),
            (2, "angle", "float", false, false, false, false, false),
            (3, "x", "int", false, false, false, false, false),
            (4, "y", "int", false, false, false, false, false),
            (5, "color", "int", false, false, false, false, false),
            (
                6,
                "font_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (7, "text", "string", false, false, true, false, false),
            (8, "options", "array", false, false, false, true, false),
        ],
    ),
    (
        "imagegammacorrect",
        &[
            (1, "input_gamma", "float", false, false, false, false, false),
            (
                2,
                "output_gamma",
                "float",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "imagegd",
        &[(1, "file", "?string", false, true, true, false, false)],
    ),
    (
        "imagegd2",
        &[
            (1, "file", "?string", false, true, true, false, false),
            (2, "chunk_size", "int", false, false, false, false, false),
            (3, "mode", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagegrabwindow",
        &[
            (0, "handle", "int", false, false, false, false, false),
            (1, "client_area", "bool", false, false, false, false, false),
        ],
    ),
    (
        "imageinterlace",
        &[(1, "enable", "?bool", false, true, false, false, false)],
    ),
    (
        "imagejpeg",
        &[(2, "quality", "int", false, false, false, false, false)],
    ),
    (
        "imagelayereffect",
        &[(1, "effect", "int", false, false, false, false, false)],
    ),
    (
        "imageline",
        &[
            (1, "x1", "int", false, false, false, false, false),
            (2, "y1", "int", false, false, false, false, false),
            (3, "x2", "int", false, false, false, false, false),
            (4, "y2", "int", false, false, false, false, false),
            (5, "color", "int", false, false, false, false, false),
        ],
    ),
    (
        "imageloadfont",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "imageopenpolygon",
        &[
            (1, "points", "array", false, false, false, true, false),
            (
                2,
                "num_points_or_color",
                "int",
                false,
                false,
                false,
                false,
                false,
            ),
            (3, "color", "?int", false, true, false, false, false),
        ],
    ),
    (
        "imagepng",
        &[
            (2, "quality", "int", false, false, false, false, false),
            (3, "filters", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagepolygon",
        &[
            (1, "points", "array", false, false, false, true, false),
            (
                2,
                "num_points_or_color",
                "int",
                false,
                false,
                false,
                false,
                false,
            ),
            (3, "color", "?int", false, true, false, false, false),
        ],
    ),
    (
        "imagerectangle",
        &[
            (1, "x1", "int", false, false, false, false, false),
            (2, "y1", "int", false, false, false, false, false),
            (3, "x2", "int", false, false, false, false, false),
            (4, "y2", "int", false, false, false, false, false),
            (5, "color", "int", false, false, false, false, false),
        ],
    ),
    (
        "imageresolution",
        &[
            (1, "resolution_x", "?int", false, true, false, false, false),
            (2, "resolution_y", "?int", false, true, false, false, false),
        ],
    ),
    (
        "imagerotate",
        &[
            (1, "angle", "float", false, false, false, false, false),
            (
                2,
                "background_color",
                "int",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "imagesavealpha",
        &[(1, "enable", "bool", false, false, false, false, false)],
    ),
    (
        "imagescale",
        &[
            (1, "width", "int", false, false, false, false, false),
            (2, "height", "int", false, false, false, false, false),
            (3, "mode", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagesetclip",
        &[
            (1, "x1", "int", false, false, false, false, false),
            (2, "y1", "int", false, false, false, false, false),
            (3, "x2", "int", false, false, false, false, false),
            (4, "y2", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagesetinterpolation",
        &[(1, "method", "int", false, false, false, false, false)],
    ),
    (
        "imagesetpixel",
        &[
            (1, "x", "int", false, false, false, false, false),
            (2, "y", "int", false, false, false, false, false),
            (3, "color", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagesetstyle",
        &[(1, "style", "array", false, false, false, true, false)],
    ),
    (
        "imagesetthickness",
        &[(1, "thickness", "int", false, false, false, false, false)],
    ),
    (
        "imagestring",
        &[
            (2, "x", "int", false, false, false, false, false),
            (3, "y", "int", false, false, false, false, false),
            (4, "string", "string", false, false, true, false, false),
            (5, "color", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagestringup",
        &[
            (2, "x", "int", false, false, false, false, false),
            (3, "y", "int", false, false, false, false, false),
            (4, "string", "string", false, false, true, false, false),
            (5, "color", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagetruecolortopalette",
        &[
            (1, "dither", "bool", false, false, false, false, false),
            (2, "num_colors", "int", false, false, false, false, false),
        ],
    ),
    (
        "imagettfbbox",
        &[
            (0, "size", "float", false, false, false, false, false),
            (1, "angle", "float", false, false, false, false, false),
            (
                2,
                "font_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (3, "string", "string", false, false, true, false, false),
            (4, "options", "array", false, false, false, true, false),
        ],
    ),
    (
        "imagettftext",
        &[
            (1, "size", "float", false, false, false, false, false),
            (2, "angle", "float", false, false, false, false, false),
            (3, "x", "int", false, false, false, false, false),
            (4, "y", "int", false, false, false, false, false),
            (5, "color", "int", false, false, false, false, false),
            (
                6,
                "font_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (7, "text", "string", false, false, true, false, false),
            (8, "options", "array", false, false, false, true, false),
        ],
    ),
    (
        "imagewbmp",
        &[(
            2,
            "foreground_color",
            "?int",
            false,
            true,
            false,
            false,
            false,
        )],
    ),
    (
        "imagewebp",
        &[(2, "quality", "int", false, false, false, false, false)],
    ),
    (
        "imagexbm",
        &[
            (1, "filename", "?string", false, true, true, false, false),
            (
                2,
                "foreground_color",
                "?int",
                false,
                true,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "implode",
        &[
            (
                0,
                "separator",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (1, "array", "?array", false, true, false, true, false),
        ],
    ),
    (
        "in_array",
        &[
            (1, "haystack", "array", false, false, false, true, false),
            (2, "strict", "bool", false, false, false, false, false),
        ],
    ),
    (
        "inet_ntop",
        &[(0, "ip", "string", false, false, true, false, false)],
    ),
    (
        "inet_pton",
        &[(0, "ip", "string", false, false, true, false, false)],
    ),
    (
        "inflate_add",
        &[
            (1, "data", "string", false, false, true, false, false),
            (2, "flush_mode", "int", false, false, false, false, false),
        ],
    ),
    (
        "inflate_init",
        &[(0, "encoding", "int", false, false, false, false, false)],
    ),
    (
        "ini_alter",
        &[(0, "option", "string", false, false, true, false, false)],
    ),
    (
        "ini_get",
        &[(0, "option", "string", false, false, true, false, false)],
    ),
    (
        "ini_get_all",
        &[
            (0, "extension", "?string", false, true, true, false, false),
            (1, "details", "bool", false, false, false, false, false),
        ],
    ),
    (
        "ini_parse_quantity",
        &[(0, "shorthand", "string", false, false, true, false, false)],
    ),
    (
        "ini_restore",
        &[(0, "option", "string", false, false, true, false, false)],
    ),
    (
        "ini_set",
        &[(0, "option", "string", false, false, true, false, false)],
    ),
    (
        "intdiv",
        &[
            (0, "num1", "int", false, false, false, false, false),
            (1, "num2", "int", false, false, false, false, false),
        ],
    ),
    (
        "interface_exists",
        &[
            (0, "interface", "string", false, false, true, false, false),
            (1, "autoload", "bool", false, false, false, false, false),
        ],
    ),
    (
        "intl_error_name",
        &[(0, "errorCode", "int", false, false, false, false, false)],
    ),
    (
        "intl_is_failure",
        &[(0, "errorCode", "int", false, false, false, false, false)],
    ),
    (
        "intlcal_add",
        &[
            (1, "field", "int", false, false, false, false, false),
            (2, "value", "int", false, false, false, false, false),
        ],
    ),
    (
        "intlcal_clear",
        &[(1, "field", "?int", false, true, false, false, false)],
    ),
    (
        "intlcal_create_instance",
        &[(1, "locale", "?string", false, true, true, false, false)],
    ),
    (
        "intlcal_field_difference",
        &[
            (1, "timestamp", "float", false, false, false, false, false),
            (2, "field", "int", false, false, false, false, false),
        ],
    ),
    (
        "intlcal_from_date_time",
        &[(1, "locale", "?string", false, true, true, false, false)],
    ),
    (
        "intlcal_get",
        &[(1, "field", "int", false, false, false, false, false)],
    ),
    (
        "intlcal_get_actual_maximum",
        &[(1, "field", "int", false, false, false, false, false)],
    ),
    (
        "intlcal_get_actual_minimum",
        &[(1, "field", "int", false, false, false, false, false)],
    ),
    (
        "intlcal_get_day_of_week_type",
        &[(1, "dayOfWeek", "int", false, false, false, false, false)],
    ),
    (
        "intlcal_get_greatest_minimum",
        &[(1, "field", "int", false, false, false, false, false)],
    ),
    (
        "intlcal_get_keyword_values_for_locale",
        &[
            (0, "keyword", "string", false, false, true, false, false),
            (1, "locale", "string", false, false, true, false, false),
            (2, "onlyCommon", "bool", false, false, false, false, false),
        ],
    ),
    (
        "intlcal_get_least_maximum",
        &[(1, "field", "int", false, false, false, false, false)],
    ),
    (
        "intlcal_get_locale",
        &[(1, "type", "int", false, false, false, false, false)],
    ),
    (
        "intlcal_get_maximum",
        &[(1, "field", "int", false, false, false, false, false)],
    ),
    (
        "intlcal_get_minimum",
        &[(1, "field", "int", false, false, false, false, false)],
    ),
    (
        "intlcal_get_weekend_transition",
        &[(1, "dayOfWeek", "int", false, false, false, false, false)],
    ),
    (
        "intlcal_is_set",
        &[(1, "field", "int", false, false, false, false, false)],
    ),
    (
        "intlcal_is_weekend",
        &[(1, "timestamp", "?float", false, true, false, false, false)],
    ),
    (
        "intlcal_roll",
        &[(1, "field", "int", false, false, false, false, false)],
    ),
    (
        "intlcal_set",
        &[
            (1, "year", "int", false, false, false, false, false),
            (2, "month", "int", false, false, false, false, false),
            (3, "dayOfMonth", "int", false, false, false, false, false),
            (4, "hour", "int", false, false, false, false, false),
            (5, "minute", "int", false, false, false, false, false),
            (6, "second", "int", false, false, false, false, false),
        ],
    ),
    (
        "intlcal_set_first_day_of_week",
        &[(1, "dayOfWeek", "int", false, false, false, false, false)],
    ),
    (
        "intlcal_set_lenient",
        &[(1, "lenient", "bool", false, false, false, false, false)],
    ),
    (
        "intlcal_set_minimal_days_in_first_week",
        &[(1, "days", "int", false, false, false, false, false)],
    ),
    (
        "intlcal_set_repeated_wall_time_option",
        &[(1, "option", "int", false, false, false, false, false)],
    ),
    (
        "intlcal_set_skipped_wall_time_option",
        &[(1, "option", "int", false, false, false, false, false)],
    ),
    (
        "intlcal_set_time",
        &[(1, "timestamp", "float", false, false, false, false, false)],
    ),
    (
        "intlgregcal_is_leap_year",
        &[(1, "year", "int", false, false, false, false, false)],
    ),
    (
        "intlgregcal_set_gregorian_change",
        &[(1, "timestamp", "float", false, false, false, false, false)],
    ),
    (
        "intltz_count_equivalent_ids",
        &[(0, "timezoneId", "string", false, false, true, false, false)],
    ),
    (
        "intltz_create_enumeration",
        &[(
            0,
            "countryOrRawOffset",
            "string|int|null",
            false,
            true,
            true,
            false,
            false,
        )],
    ),
    (
        "intltz_create_time_zone",
        &[(0, "timezoneId", "string", false, false, true, false, false)],
    ),
    (
        "intltz_create_time_zone_id_enumeration",
        &[
            (0, "type", "int", false, false, false, false, false),
            (1, "region", "?string", false, true, true, false, false),
            (2, "rawOffset", "?int", false, true, false, false, false),
        ],
    ),
    (
        "intltz_get_canonical_id",
        &[(0, "timezoneId", "string", false, false, true, false, false)],
    ),
    (
        "intltz_get_display_name",
        &[
            (1, "dst", "bool", false, false, false, false, false),
            (2, "style", "int", false, false, false, false, false),
            (3, "locale", "?string", false, true, true, false, false),
        ],
    ),
    (
        "intltz_get_equivalent_id",
        &[
            (0, "timezoneId", "string", false, false, true, false, false),
            (1, "offset", "int", false, false, false, false, false),
        ],
    ),
    (
        "intltz_get_iana_id",
        &[(0, "timezoneId", "string", false, false, true, false, false)],
    ),
    (
        "intltz_get_id_for_windows_id",
        &[
            (0, "timezoneId", "string", false, false, true, false, false),
            (1, "region", "?string", false, true, true, false, false),
        ],
    ),
    (
        "intltz_get_offset",
        &[
            (1, "timestamp", "float", false, false, false, false, false),
            (2, "local", "bool", false, false, false, false, false),
        ],
    ),
    (
        "intltz_get_region",
        &[(0, "timezoneId", "string", false, false, true, false, false)],
    ),
    (
        "intltz_get_windows_id",
        &[(0, "timezoneId", "string", false, false, true, false, false)],
    ),
    (
        "intval",
        &[(1, "base", "int", false, false, false, false, false)],
    ),
    (
        "ip2long",
        &[(0, "ip", "string", false, false, true, false, false)],
    ),
    (
        "iptcembed",
        &[
            (0, "iptc_data", "string", false, false, true, false, false),
            (1, "filename", "string", false, false, true, false, false),
            (2, "spool", "int", false, false, false, false, false),
        ],
    ),
    (
        "iptcparse",
        &[(0, "iptc_block", "string", false, false, true, false, false)],
    ),
    (
        "is_a",
        &[
            (1, "class", "string", false, false, true, false, false),
            (2, "allow_string", "bool", false, false, false, false, false),
        ],
    ),
    (
        "is_callable",
        &[(1, "syntax_only", "bool", false, false, false, false, false)],
    ),
    (
        "is_dir",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "is_executable",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "is_file",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "is_finite",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "is_infinite",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "is_link",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "is_nan",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "is_readable",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "is_subclass_of",
        &[
            (1, "class", "string", false, false, true, false, false),
            (2, "allow_string", "bool", false, false, false, false, false),
        ],
    ),
    (
        "is_uploaded_file",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "is_writable",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "is_writeable",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "iterator_apply",
        &[(2, "args", "?array", false, true, false, true, false)],
    ),
    (
        "iterator_to_array",
        &[(
            1,
            "preserve_keys",
            "bool",
            false,
            false,
            false,
            false,
            false,
        )],
    ),
    (
        "jddayofweek",
        &[
            (0, "julian_day", "int", false, false, false, false, false),
            (1, "mode", "int", false, false, false, false, false),
        ],
    ),
    (
        "jdmonthname",
        &[
            (0, "julian_day", "int", false, false, false, false, false),
            (1, "mode", "int", false, false, false, false, false),
        ],
    ),
    (
        "jdtofrench",
        &[(0, "julian_day", "int", false, false, false, false, false)],
    ),
    (
        "jdtogregorian",
        &[(0, "julian_day", "int", false, false, false, false, false)],
    ),
    (
        "jdtojewish",
        &[
            (0, "julian_day", "int", false, false, false, false, false),
            (1, "hebrew", "bool", false, false, false, false, false),
            (2, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "jdtojulian",
        &[(0, "julian_day", "int", false, false, false, false, false)],
    ),
    (
        "jdtounix",
        &[(0, "julian_day", "int", false, false, false, false, false)],
    ),
    (
        "jewishtojd",
        &[
            (0, "month", "int", false, false, false, false, false),
            (1, "day", "int", false, false, false, false, false),
            (2, "year", "int", false, false, false, false, false),
        ],
    ),
    (
        "join",
        &[
            (
                0,
                "separator",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (1, "array", "?array", false, true, false, true, false),
        ],
    ),
    (
        "json_decode",
        &[
            (0, "json", "string", false, false, true, false, false),
            (1, "associative", "?bool", false, true, false, false, false),
            (2, "depth", "int", false, false, false, false, false),
            (3, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "json_encode",
        &[
            (1, "flags", "int", false, false, false, false, false),
            (2, "depth", "int", false, false, false, false, false),
        ],
    ),
    (
        "json_validate",
        &[
            (0, "json", "string", false, false, true, false, false),
            (1, "depth", "int", false, false, false, false, false),
            (2, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "juliantojd",
        &[
            (0, "month", "int", false, false, false, false, false),
            (1, "day", "int", false, false, false, false, false),
            (2, "year", "int", false, false, false, false, false),
        ],
    ),
    (
        "key_exists",
        &[(1, "array", "array", false, false, false, true, false)],
    ),
    (
        "krsort",
        &[
            (0, "array", "array", false, false, false, true, true),
            (1, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "ksort",
        &[
            (0, "array", "array", false, false, false, true, true),
            (1, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "lcfirst",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "lchgrp",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "group", "string|int", false, false, true, false, false),
        ],
    ),
    (
        "lchown",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "user", "string|int", false, false, true, false, false),
        ],
    ),
    (
        "levenshtein",
        &[
            (0, "string1", "string", false, false, true, false, false),
            (1, "string2", "string", false, false, true, false, false),
            (
                2,
                "insertion_cost",
                "int",
                false,
                false,
                false,
                false,
                false,
            ),
            (
                3,
                "replacement_cost",
                "int",
                false,
                false,
                false,
                false,
                false,
            ),
            (4, "deletion_cost", "int", false, false, false, false, false),
        ],
    ),
    (
        "libxml_disable_entity_loader",
        &[(0, "disable", "bool", false, false, false, false, false)],
    ),
    (
        "libxml_use_internal_errors",
        &[(0, "use_errors", "?bool", false, true, false, false, false)],
    ),
    (
        "link",
        &[
            (0, "target", "string", false, false, true, false, false),
            (1, "link", "string", false, false, true, false, false),
        ],
    ),
    (
        "linkinfo",
        &[(0, "path", "string", false, false, true, false, false)],
    ),
    (
        "locale_accept_from_http",
        &[(0, "header", "string", false, false, true, false, false)],
    ),
    (
        "locale_add_likely_subtags",
        &[(0, "locale", "string", false, false, true, false, false)],
    ),
    (
        "locale_canonicalize",
        &[(0, "locale", "string", false, false, true, false, false)],
    ),
    (
        "locale_compose",
        &[(0, "subtags", "array", false, false, false, true, false)],
    ),
    (
        "locale_filter_matches",
        &[
            (0, "languageTag", "string", false, false, true, false, false),
            (1, "locale", "string", false, false, true, false, false),
            (2, "canonicalize", "bool", false, false, false, false, false),
        ],
    ),
    (
        "locale_get_all_variants",
        &[(0, "locale", "string", false, false, true, false, false)],
    ),
    (
        "locale_get_display_language",
        &[
            (0, "locale", "string", false, false, true, false, false),
            (
                1,
                "displayLocale",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "locale_get_display_name",
        &[
            (0, "locale", "string", false, false, true, false, false),
            (
                1,
                "displayLocale",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "locale_get_display_region",
        &[
            (0, "locale", "string", false, false, true, false, false),
            (
                1,
                "displayLocale",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "locale_get_display_script",
        &[
            (0, "locale", "string", false, false, true, false, false),
            (
                1,
                "displayLocale",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "locale_get_display_variant",
        &[
            (0, "locale", "string", false, false, true, false, false),
            (
                1,
                "displayLocale",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "locale_get_keywords",
        &[(0, "locale", "string", false, false, true, false, false)],
    ),
    (
        "locale_get_primary_language",
        &[(0, "locale", "string", false, false, true, false, false)],
    ),
    (
        "locale_get_region",
        &[(0, "locale", "string", false, false, true, false, false)],
    ),
    (
        "locale_get_script",
        &[(0, "locale", "string", false, false, true, false, false)],
    ),
    (
        "locale_is_right_to_left",
        &[(0, "locale", "string", false, false, true, false, false)],
    ),
    (
        "locale_lookup",
        &[
            (0, "languageTag", "array", false, false, false, true, false),
            (1, "locale", "string", false, false, true, false, false),
            (2, "canonicalize", "bool", false, false, false, false, false),
            (
                3,
                "defaultLocale",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "locale_minimize_subtags",
        &[(0, "locale", "string", false, false, true, false, false)],
    ),
    (
        "locale_parse",
        &[(0, "locale", "string", false, false, true, false, false)],
    ),
    (
        "locale_set_default",
        &[(0, "locale", "string", false, false, true, false, false)],
    ),
    (
        "localtime",
        &[
            (0, "timestamp", "?int", false, true, false, false, false),
            (1, "associative", "bool", false, false, false, false, false),
        ],
    ),
    (
        "log",
        &[
            (0, "num", "float", false, false, false, false, false),
            (1, "base", "float", false, false, false, false, false),
        ],
    ),
    (
        "log10",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "log1p",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "long2ip",
        &[(0, "ip", "int", false, false, false, false, false)],
    ),
    (
        "lstat",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "ltrim",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "characters", "string", false, false, true, false, false),
        ],
    ),
    (
        "mail",
        &[
            (0, "to", "string", false, false, true, false, false),
            (1, "subject", "string", false, false, true, false, false),
            (2, "message", "string", false, false, true, false, false),
            (
                3,
                "additional_headers",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (
                4,
                "additional_params",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "mb_check_encoding",
        &[
            (
                0,
                "value",
                "array|string|null",
                false,
                true,
                true,
                true,
                false,
            ),
            (1, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_chr",
        &[
            (0, "codepoint", "int", false, false, false, false, false),
            (1, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_convert_case",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "mode", "int", false, false, false, false, false),
            (2, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_convert_encoding",
        &[
            (0, "string", "array|string", false, false, true, true, false),
            (1, "to_encoding", "string", false, false, true, false, false),
            (
                2,
                "from_encoding",
                "array|string|null",
                false,
                true,
                true,
                true,
                false,
            ),
        ],
    ),
    (
        "mb_convert_kana",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "mode", "string", false, false, true, false, false),
            (2, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_convert_variables",
        &[
            (0, "to_encoding", "string", false, false, true, false, false),
            (
                1,
                "from_encoding",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
        ],
    ),
    (
        "mb_decode_mimeheader",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "mb_decode_numericentity",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "map", "array", false, false, false, true, false),
            (2, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_detect_encoding",
        &[
            (0, "string", "string", false, false, true, false, false),
            (
                1,
                "encodings",
                "array|string|null",
                false,
                true,
                true,
                true,
                false,
            ),
            (2, "strict", "bool", false, false, false, false, false),
        ],
    ),
    (
        "mb_detect_order",
        &[(
            0,
            "encoding",
            "array|string|null",
            false,
            true,
            true,
            true,
            false,
        )],
    ),
    (
        "mb_encode_mimeheader",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "charset", "?string", false, true, true, false, false),
            (
                2,
                "transfer_encoding",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
            (3, "newline", "string", false, false, true, false, false),
            (4, "indent", "int", false, false, false, false, false),
        ],
    ),
    (
        "mb_encode_numericentity",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "map", "array", false, false, false, true, false),
            (2, "encoding", "?string", false, true, true, false, false),
            (3, "hex", "bool", false, false, false, false, false),
        ],
    ),
    (
        "mb_encoding_aliases",
        &[(0, "encoding", "string", false, false, true, false, false)],
    ),
    (
        "mb_ereg",
        &[
            (0, "pattern", "string", false, false, true, false, false),
            (1, "string", "string", false, false, true, false, false),
        ],
    ),
    (
        "mb_ereg_match",
        &[
            (0, "pattern", "string", false, false, true, false, false),
            (1, "string", "string", false, false, true, false, false),
            (2, "options", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_ereg_replace",
        &[
            (0, "pattern", "string", false, false, true, false, false),
            (1, "replacement", "string", false, false, true, false, false),
            (2, "string", "string", false, false, true, false, false),
            (3, "options", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_ereg_replace_callback",
        &[
            (0, "pattern", "string", false, false, true, false, false),
            (2, "string", "string", false, false, true, false, false),
            (3, "options", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_ereg_search",
        &[
            (0, "pattern", "?string", false, true, true, false, false),
            (1, "options", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_ereg_search_init",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "pattern", "?string", false, true, true, false, false),
            (2, "options", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_ereg_search_pos",
        &[
            (0, "pattern", "?string", false, true, true, false, false),
            (1, "options", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_ereg_search_regs",
        &[
            (0, "pattern", "?string", false, true, true, false, false),
            (1, "options", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_ereg_search_setpos",
        &[(0, "offset", "int", false, false, false, false, false)],
    ),
    (
        "mb_eregi",
        &[
            (0, "pattern", "string", false, false, true, false, false),
            (1, "string", "string", false, false, true, false, false),
        ],
    ),
    (
        "mb_eregi_replace",
        &[
            (0, "pattern", "string", false, false, true, false, false),
            (1, "replacement", "string", false, false, true, false, false),
            (2, "string", "string", false, false, true, false, false),
            (3, "options", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_get_info",
        &[(0, "type", "string", false, false, true, false, false)],
    ),
    (
        "mb_http_input",
        &[(0, "type", "?string", false, true, true, false, false)],
    ),
    (
        "mb_http_output",
        &[(0, "encoding", "?string", false, true, true, false, false)],
    ),
    (
        "mb_internal_encoding",
        &[(0, "encoding", "?string", false, true, true, false, false)],
    ),
    (
        "mb_language",
        &[(0, "language", "?string", false, true, true, false, false)],
    ),
    (
        "mb_lcfirst",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_ltrim",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "characters", "?string", false, true, true, false, false),
            (2, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_ord",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_output_handler",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "status", "int", false, false, false, false, false),
        ],
    ),
    (
        "mb_parse_str",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "mb_preferred_mime_name",
        &[(0, "encoding", "string", false, false, true, false, false)],
    ),
    (
        "mb_regex_encoding",
        &[(0, "encoding", "?string", false, true, true, false, false)],
    ),
    (
        "mb_regex_set_options",
        &[(0, "options", "?string", false, true, true, false, false)],
    ),
    (
        "mb_rtrim",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "characters", "?string", false, true, true, false, false),
            (2, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_scrub",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_send_mail",
        &[
            (0, "to", "string", false, false, true, false, false),
            (1, "subject", "string", false, false, true, false, false),
            (2, "message", "string", false, false, true, false, false),
            (
                3,
                "additional_headers",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (
                4,
                "additional_params",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "mb_split",
        &[
            (0, "pattern", "string", false, false, true, false, false),
            (1, "string", "string", false, false, true, false, false),
            (2, "limit", "int", false, false, false, false, false),
        ],
    ),
    (
        "mb_str_pad",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "length", "int", false, false, false, false, false),
            (2, "pad_string", "string", false, false, true, false, false),
            (3, "pad_type", "int", false, false, false, false, false),
            (4, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_str_split",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "length", "int", false, false, false, false, false),
            (2, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_strcut",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "start", "int", false, false, false, false, false),
            (2, "length", "?int", false, true, false, false, false),
            (3, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_strimwidth",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "start", "int", false, false, false, false, false),
            (2, "width", "int", false, false, false, false, false),
            (3, "trim_marker", "string", false, false, true, false, false),
            (4, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_stripos",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (2, "offset", "int", false, false, false, false, false),
            (3, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_stristr",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (
                2,
                "before_needle",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
            (3, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_strlen",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_strpos",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (2, "offset", "int", false, false, false, false, false),
            (3, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_strrchr",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (
                2,
                "before_needle",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
            (3, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_strrichr",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (
                2,
                "before_needle",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
            (3, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_strripos",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (2, "offset", "int", false, false, false, false, false),
            (3, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_strrpos",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (2, "offset", "int", false, false, false, false, false),
            (3, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_strstr",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (
                2,
                "before_needle",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
            (3, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_strtolower",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_strtoupper",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_strwidth",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_substitute_character",
        &[(
            0,
            "substitute_character",
            "string|int|null",
            false,
            true,
            true,
            false,
            false,
        )],
    ),
    (
        "mb_substr",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "start", "int", false, false, false, false, false),
            (2, "length", "?int", false, true, false, false, false),
            (3, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_substr_count",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (2, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_trim",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "characters", "?string", false, true, true, false, false),
            (2, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mb_ucfirst",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "md5",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "binary", "bool", false, false, false, false, false),
        ],
    ),
    (
        "md5_file",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "binary", "bool", false, false, false, false, false),
        ],
    ),
    (
        "memory_get_peak_usage",
        &[(0, "real_usage", "bool", false, false, false, false, false)],
    ),
    (
        "memory_get_usage",
        &[(0, "real_usage", "bool", false, false, false, false, false)],
    ),
    (
        "metaphone",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "max_phonemes", "int", false, false, false, false, false),
        ],
    ),
    (
        "method_exists",
        &[(1, "method", "string", false, false, true, false, false)],
    ),
    (
        "mhash",
        &[
            (0, "algo", "int", false, false, false, false, false),
            (1, "data", "string", false, false, true, false, false),
            (2, "key", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mhash_get_block_size",
        &[(0, "algo", "int", false, false, false, false, false)],
    ),
    (
        "mhash_get_hash_name",
        &[(0, "algo", "int", false, false, false, false, false)],
    ),
    (
        "mhash_keygen_s2k",
        &[
            (0, "algo", "int", false, false, false, false, false),
            (1, "password", "string", false, false, true, false, false),
            (2, "salt", "string", false, false, true, false, false),
            (3, "length", "int", false, false, false, false, false),
        ],
    ),
    (
        "microtime",
        &[(0, "as_float", "bool", false, false, false, false, false)],
    ),
    (
        "mkdir",
        &[
            (0, "directory", "string", false, false, true, false, false),
            (1, "permissions", "int", false, false, false, false, false),
            (2, "recursive", "bool", false, false, false, false, false),
        ],
    ),
    (
        "mktime",
        &[
            (0, "hour", "int", false, false, false, false, false),
            (1, "minute", "?int", false, true, false, false, false),
            (2, "second", "?int", false, true, false, false, false),
            (3, "month", "?int", false, true, false, false, false),
            (4, "day", "?int", false, true, false, false, false),
            (5, "year", "?int", false, true, false, false, false),
        ],
    ),
    (
        "move_uploaded_file",
        &[
            (0, "from", "string", false, false, true, false, false),
            (1, "to", "string", false, false, true, false, false),
        ],
    ),
    (
        "msg_get_queue",
        &[
            (0, "key", "int", false, false, false, false, false),
            (1, "permissions", "int", false, false, false, false, false),
        ],
    ),
    (
        "msg_queue_exists",
        &[(0, "key", "int", false, false, false, false, false)],
    ),
    (
        "msg_receive",
        &[
            (
                1,
                "desired_message_type",
                "int",
                false,
                false,
                false,
                false,
                false,
            ),
            (
                3,
                "max_message_size",
                "int",
                false,
                false,
                false,
                false,
                false,
            ),
            (5, "unserialize", "bool", false, false, false, false, false),
            (6, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "msg_send",
        &[
            (1, "message_type", "int", false, false, false, false, false),
            (3, "serialize", "bool", false, false, false, false, false),
            (4, "blocking", "bool", false, false, false, false, false),
        ],
    ),
    (
        "msg_set_queue",
        &[(1, "data", "array", false, false, false, true, false)],
    ),
    (
        "msgfmt_create",
        &[
            (0, "locale", "string", false, false, true, false, false),
            (1, "pattern", "string", false, false, true, false, false),
        ],
    ),
    (
        "msgfmt_format",
        &[(1, "values", "array", false, false, false, true, false)],
    ),
    (
        "msgfmt_format_message",
        &[
            (0, "locale", "string", false, false, true, false, false),
            (1, "pattern", "string", false, false, true, false, false),
            (2, "values", "array", false, false, false, true, false),
        ],
    ),
    (
        "msgfmt_parse",
        &[(1, "string", "string", false, false, true, false, false)],
    ),
    (
        "msgfmt_parse_message",
        &[
            (0, "locale", "string", false, false, true, false, false),
            (1, "pattern", "string", false, false, true, false, false),
            (2, "message", "string", false, false, true, false, false),
        ],
    ),
    (
        "msgfmt_set_pattern",
        &[(1, "pattern", "string", false, false, true, false, false)],
    ),
    (
        "mysqli_autocommit",
        &[(1, "enable", "bool", false, false, false, false, false)],
    ),
    (
        "mysqli_begin_transaction",
        &[
            (1, "flags", "int", false, false, false, false, false),
            (2, "name", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mysqli_change_user",
        &[
            (1, "username", "string", false, false, true, false, false),
            (2, "password", "string", false, false, true, false, false),
            (3, "database", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mysqli_commit",
        &[
            (1, "flags", "int", false, false, false, false, false),
            (2, "name", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mysqli_connect",
        &[
            (0, "hostname", "?string", false, true, true, false, false),
            (1, "username", "?string", false, true, true, false, false),
            (2, "password", "?string", false, true, true, false, false),
            (3, "database", "?string", false, true, true, false, false),
            (4, "port", "?int", false, true, false, false, false),
            (5, "socket", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mysqli_data_seek",
        &[(1, "offset", "int", false, false, false, false, false)],
    ),
    (
        "mysqli_debug",
        &[(0, "options", "string", false, false, true, false, false)],
    ),
    (
        "mysqli_escape_string",
        &[(1, "string", "string", false, false, true, false, false)],
    ),
    (
        "mysqli_execute",
        &[(1, "params", "?array", false, true, false, true, false)],
    ),
    (
        "mysqli_execute_query",
        &[
            (1, "query", "string", false, false, true, false, false),
            (2, "params", "?array", false, true, false, true, false),
        ],
    ),
    (
        "mysqli_fetch_all",
        &[(1, "mode", "int", false, false, false, false, false)],
    ),
    (
        "mysqli_fetch_array",
        &[(1, "mode", "int", false, false, false, false, false)],
    ),
    (
        "mysqli_fetch_column",
        &[(1, "column", "int", false, false, false, false, false)],
    ),
    (
        "mysqli_fetch_field_direct",
        &[(1, "index", "int", false, false, false, false, false)],
    ),
    (
        "mysqli_fetch_object",
        &[
            (1, "class", "string", false, false, true, false, false),
            (
                2,
                "constructor_args",
                "array",
                false,
                false,
                false,
                true,
                false,
            ),
        ],
    ),
    (
        "mysqli_field_seek",
        &[(1, "index", "int", false, false, false, false, false)],
    ),
    (
        "mysqli_kill",
        &[(1, "process_id", "int", false, false, false, false, false)],
    ),
    (
        "mysqli_multi_query",
        &[(1, "query", "string", false, false, true, false, false)],
    ),
    (
        "mysqli_options",
        &[(1, "option", "int", false, false, false, false, false)],
    ),
    (
        "mysqli_poll",
        &[
            (2, "reject", "array", false, false, false, true, true),
            (3, "seconds", "int", false, false, false, false, false),
            (4, "microseconds", "int", false, false, false, false, false),
        ],
    ),
    (
        "mysqli_prepare",
        &[(1, "query", "string", false, false, true, false, false)],
    ),
    (
        "mysqli_query",
        &[
            (1, "query", "string", false, false, true, false, false),
            (2, "result_mode", "int", false, false, false, false, false),
        ],
    ),
    (
        "mysqli_real_connect",
        &[
            (1, "hostname", "?string", false, true, true, false, false),
            (2, "username", "?string", false, true, true, false, false),
            (3, "password", "?string", false, true, true, false, false),
            (4, "database", "?string", false, true, true, false, false),
            (5, "port", "?int", false, true, false, false, false),
            (6, "socket", "?string", false, true, true, false, false),
            (7, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "mysqli_real_escape_string",
        &[(1, "string", "string", false, false, true, false, false)],
    ),
    (
        "mysqli_real_query",
        &[(1, "query", "string", false, false, true, false, false)],
    ),
    (
        "mysqli_refresh",
        &[(1, "flags", "int", false, false, false, false, false)],
    ),
    (
        "mysqli_release_savepoint",
        &[(1, "name", "string", false, false, true, false, false)],
    ),
    (
        "mysqli_report",
        &[(0, "flags", "int", false, false, false, false, false)],
    ),
    (
        "mysqli_rollback",
        &[
            (1, "flags", "int", false, false, false, false, false),
            (2, "name", "?string", false, true, true, false, false),
        ],
    ),
    (
        "mysqli_savepoint",
        &[(1, "name", "string", false, false, true, false, false)],
    ),
    (
        "mysqli_select_db",
        &[(1, "database", "string", false, false, true, false, false)],
    ),
    (
        "mysqli_set_charset",
        &[(1, "charset", "string", false, false, true, false, false)],
    ),
    (
        "mysqli_set_opt",
        &[(1, "option", "int", false, false, false, false, false)],
    ),
    (
        "mysqli_ssl_set",
        &[
            (1, "key", "?string", false, true, true, false, false),
            (2, "certificate", "?string", false, true, true, false, false),
            (
                3,
                "ca_certificate",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
            (4, "ca_path", "?string", false, true, true, false, false),
            (
                5,
                "cipher_algos",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "mysqli_stmt_attr_get",
        &[(1, "attribute", "int", false, false, false, false, false)],
    ),
    (
        "mysqli_stmt_attr_set",
        &[
            (1, "attribute", "int", false, false, false, false, false),
            (2, "value", "int", false, false, false, false, false),
        ],
    ),
    (
        "mysqli_stmt_bind_param",
        &[(1, "types", "string", false, false, true, false, false)],
    ),
    (
        "mysqli_stmt_data_seek",
        &[(1, "offset", "int", false, false, false, false, false)],
    ),
    (
        "mysqli_stmt_execute",
        &[(1, "params", "?array", false, true, false, true, false)],
    ),
    (
        "mysqli_stmt_prepare",
        &[(1, "query", "string", false, false, true, false, false)],
    ),
    (
        "mysqli_stmt_send_long_data",
        &[
            (1, "param_num", "int", false, false, false, false, false),
            (2, "data", "string", false, false, true, false, false),
        ],
    ),
    (
        "mysqli_store_result",
        &[(1, "mode", "int", false, false, false, false, false)],
    ),
    (
        "natcasesort",
        &[(0, "array", "array", false, false, false, true, true)],
    ),
    (
        "natsort",
        &[(0, "array", "array", false, false, false, true, true)],
    ),
    (
        "ngettext",
        &[
            (0, "singular", "string", false, false, true, false, false),
            (1, "plural", "string", false, false, true, false, false),
            (2, "count", "int", false, false, false, false, false),
        ],
    ),
    (
        "nl2br",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "use_xhtml", "bool", false, false, false, false, false),
        ],
    ),
    (
        "nl_langinfo",
        &[(0, "item", "int", false, false, false, false, false)],
    ),
    (
        "normalizer_get_raw_decomposition",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "form", "int", false, false, false, false, false),
        ],
    ),
    (
        "normalizer_is_normalized",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "form", "int", false, false, false, false, false),
        ],
    ),
    (
        "normalizer_normalize",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "form", "int", false, false, false, false, false),
        ],
    ),
    (
        "number_format",
        &[
            (0, "num", "float", false, false, false, false, false),
            (1, "decimals", "int", false, false, false, false, false),
            (
                2,
                "decimal_separator",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
            (
                3,
                "thousands_separator",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "numfmt_create",
        &[
            (0, "locale", "string", false, false, true, false, false),
            (1, "style", "int", false, false, false, false, false),
            (2, "pattern", "?string", false, true, true, false, false),
        ],
    ),
    (
        "numfmt_format",
        &[
            (1, "num", "int|float", false, false, false, false, false),
            (2, "type", "int", false, false, false, false, false),
        ],
    ),
    (
        "numfmt_format_currency",
        &[
            (1, "amount", "float", false, false, false, false, false),
            (2, "currency", "string", false, false, true, false, false),
        ],
    ),
    (
        "numfmt_get_attribute",
        &[(1, "attribute", "int", false, false, false, false, false)],
    ),
    (
        "numfmt_get_locale",
        &[(1, "type", "int", false, false, false, false, false)],
    ),
    (
        "numfmt_get_symbol",
        &[(1, "symbol", "int", false, false, false, false, false)],
    ),
    (
        "numfmt_get_text_attribute",
        &[(1, "attribute", "int", false, false, false, false, false)],
    ),
    (
        "numfmt_parse",
        &[
            (1, "string", "string", false, false, true, false, false),
            (2, "type", "int", false, false, false, false, false),
        ],
    ),
    (
        "numfmt_parse_currency",
        &[(1, "string", "string", false, false, true, false, false)],
    ),
    (
        "numfmt_set_attribute",
        &[
            (1, "attribute", "int", false, false, false, false, false),
            (2, "value", "int|float", false, false, false, false, false),
        ],
    ),
    (
        "numfmt_set_pattern",
        &[(1, "pattern", "string", false, false, true, false, false)],
    ),
    (
        "numfmt_set_symbol",
        &[
            (1, "symbol", "int", false, false, false, false, false),
            (2, "value", "string", false, false, true, false, false),
        ],
    ),
    (
        "numfmt_set_text_attribute",
        &[
            (1, "attribute", "int", false, false, false, false, false),
            (2, "value", "string", false, false, true, false, false),
        ],
    ),
    (
        "ob_get_status",
        &[(0, "full_status", "bool", false, false, false, false, false)],
    ),
    (
        "ob_gzhandler",
        &[
            (0, "data", "string", false, false, true, false, false),
            (1, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "ob_implicit_flush",
        &[(0, "enable", "bool", false, false, false, false, false)],
    ),
    (
        "ob_start",
        &[
            (1, "chunk_size", "int", false, false, false, false, false),
            (2, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "octdec",
        &[(
            0,
            "octal_string",
            "string",
            false,
            false,
            true,
            false,
            false,
        )],
    ),
    (
        "opcache_compile_file",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "opcache_get_status",
        &[(
            0,
            "include_scripts",
            "bool",
            false,
            false,
            false,
            false,
            false,
        )],
    ),
    (
        "opcache_invalidate",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "force", "bool", false, false, false, false, false),
        ],
    ),
    (
        "opcache_is_script_cached",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "opcache_is_script_cached_in_file_cache",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "opendir",
        &[(0, "directory", "string", false, false, true, false, false)],
    ),
    (
        "openlog",
        &[
            (0, "prefix", "string", false, false, true, false, false),
            (1, "flags", "int", false, false, false, false, false),
            (2, "facility", "int", false, false, false, false, false),
        ],
    ),
    (
        "openssl_cipher_iv_length",
        &[(0, "cipher_algo", "string", false, false, true, false, false)],
    ),
    (
        "openssl_cipher_key_length",
        &[(0, "cipher_algo", "string", false, false, true, false, false)],
    ),
    (
        "openssl_cms_decrypt",
        &[
            (
                0,
                "input_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                1,
                "output_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (4, "encoding", "int", false, false, false, false, false),
        ],
    ),
    (
        "openssl_cms_encrypt",
        &[
            (
                0,
                "input_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                1,
                "output_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (3, "headers", "?array", false, true, false, true, false),
            (4, "flags", "int", false, false, false, false, false),
            (5, "encoding", "int", false, false, false, false, false),
            (
                6,
                "cipher_algo",
                "string|int",
                false,
                false,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "openssl_cms_read",
        &[(
            0,
            "input_filename",
            "string",
            false,
            false,
            true,
            false,
            false,
        )],
    ),
    (
        "openssl_cms_sign",
        &[
            (
                0,
                "input_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                1,
                "output_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (4, "headers", "?array", false, true, false, true, false),
            (5, "flags", "int", false, false, false, false, false),
            (6, "encoding", "int", false, false, false, false, false),
            (
                7,
                "untrusted_certificates_filename",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "openssl_cms_verify",
        &[
            (
                0,
                "input_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (1, "flags", "int", false, false, false, false, false),
            (
                2,
                "certificates",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
            (3, "ca_info", "array", false, false, false, true, false),
            (
                4,
                "untrusted_certificates_filename",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
            (5, "content", "?string", false, true, true, false, false),
            (6, "pk7", "?string", false, true, true, false, false),
            (7, "sigfile", "?string", false, true, true, false, false),
            (8, "encoding", "int", false, false, false, false, false),
        ],
    ),
    (
        "openssl_csr_export",
        &[(2, "no_text", "bool", false, false, false, false, false)],
    ),
    (
        "openssl_csr_export_to_file",
        &[
            (
                1,
                "output_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (2, "no_text", "bool", false, false, false, false, false),
        ],
    ),
    (
        "openssl_csr_get_public_key",
        &[(1, "short_names", "bool", false, false, false, false, false)],
    ),
    (
        "openssl_csr_get_subject",
        &[(1, "short_names", "bool", false, false, false, false, false)],
    ),
    (
        "openssl_csr_new",
        &[
            (
                0,
                "distinguished_names",
                "array",
                false,
                false,
                false,
                true,
                false,
            ),
            (2, "options", "?array", false, true, false, true, false),
            (
                3,
                "extra_attributes",
                "?array",
                false,
                true,
                false,
                true,
                false,
            ),
        ],
    ),
    (
        "openssl_csr_sign",
        &[
            (3, "days", "int", false, false, false, false, false),
            (4, "options", "?array", false, true, false, true, false),
            (5, "serial", "int", false, false, false, false, false),
            (6, "serial_hex", "?string", false, true, true, false, false),
        ],
    ),
    (
        "openssl_decrypt",
        &[
            (0, "data", "string", false, false, true, false, false),
            (1, "cipher_algo", "string", false, false, true, false, false),
            (2, "passphrase", "string", false, false, true, false, false),
            (3, "options", "int", false, false, false, false, false),
            (4, "iv", "string", false, false, true, false, false),
            (5, "tag", "?string", false, true, true, false, false),
            (6, "aad", "string", false, false, true, false, false),
        ],
    ),
    (
        "openssl_dh_compute_key",
        &[(0, "public_key", "string", false, false, true, false, false)],
    ),
    (
        "openssl_digest",
        &[
            (0, "data", "string", false, false, true, false, false),
            (1, "digest_algo", "string", false, false, true, false, false),
            (2, "binary", "bool", false, false, false, false, false),
        ],
    ),
    (
        "openssl_encrypt",
        &[
            (0, "data", "string", false, false, true, false, false),
            (1, "cipher_algo", "string", false, false, true, false, false),
            (2, "passphrase", "string", false, false, true, false, false),
            (3, "options", "int", false, false, false, false, false),
            (4, "iv", "string", false, false, true, false, false),
            (6, "aad", "string", false, false, true, false, false),
            (7, "tag_length", "int", false, false, false, false, false),
        ],
    ),
    (
        "openssl_get_cipher_methods",
        &[(0, "aliases", "bool", false, false, false, false, false)],
    ),
    (
        "openssl_get_md_methods",
        &[(0, "aliases", "bool", false, false, false, false, false)],
    ),
    (
        "openssl_get_privatekey",
        &[(1, "passphrase", "?string", false, true, true, false, false)],
    ),
    (
        "openssl_open",
        &[
            (0, "data", "string", false, false, true, false, false),
            (
                2,
                "encrypted_key",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (4, "cipher_algo", "string", false, false, true, false, false),
            (5, "iv", "?string", false, true, true, false, false),
        ],
    ),
    (
        "openssl_password_hash",
        &[
            (0, "algo", "string", false, false, true, false, false),
            (1, "password", "string", false, false, true, false, false),
            (2, "options", "array", false, false, false, true, false),
        ],
    ),
    (
        "openssl_password_verify",
        &[
            (0, "algo", "string", false, false, true, false, false),
            (1, "password", "string", false, false, true, false, false),
            (2, "hash", "string", false, false, true, false, false),
        ],
    ),
    (
        "openssl_pbkdf2",
        &[
            (0, "password", "string", false, false, true, false, false),
            (1, "salt", "string", false, false, true, false, false),
            (2, "key_length", "int", false, false, false, false, false),
            (3, "iterations", "int", false, false, false, false, false),
            (4, "digest_algo", "string", false, false, true, false, false),
        ],
    ),
    (
        "openssl_pkcs12_export",
        &[
            (3, "passphrase", "string", false, false, true, false, false),
            (4, "options", "array", false, false, false, true, false),
        ],
    ),
    (
        "openssl_pkcs12_export_to_file",
        &[
            (
                1,
                "output_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (3, "passphrase", "string", false, false, true, false, false),
            (4, "options", "array", false, false, false, true, false),
        ],
    ),
    (
        "openssl_pkcs12_read",
        &[
            (0, "pkcs12", "string", false, false, true, false, false),
            (2, "passphrase", "string", false, false, true, false, false),
        ],
    ),
    (
        "openssl_pkcs7_decrypt",
        &[
            (
                0,
                "input_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                1,
                "output_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "openssl_pkcs7_encrypt",
        &[
            (
                0,
                "input_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                1,
                "output_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (3, "headers", "?array", false, true, false, true, false),
            (4, "flags", "int", false, false, false, false, false),
            (5, "cipher_algo", "int", false, false, false, false, false),
        ],
    ),
    (
        "openssl_pkcs7_read",
        &[(0, "data", "string", false, false, true, false, false)],
    ),
    (
        "openssl_pkcs7_sign",
        &[
            (
                0,
                "input_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                1,
                "output_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (4, "headers", "?array", false, true, false, true, false),
            (5, "flags", "int", false, false, false, false, false),
            (
                6,
                "untrusted_certificates_filename",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "openssl_pkcs7_verify",
        &[
            (
                0,
                "input_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (1, "flags", "int", false, false, false, false, false),
            (
                2,
                "signers_certificates_filename",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
            (3, "ca_info", "array", false, false, false, true, false),
            (
                4,
                "untrusted_certificates_filename",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
            (5, "content", "?string", false, true, true, false, false),
            (
                6,
                "output_filename",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "openssl_pkey_derive",
        &[(2, "key_length", "int", false, false, false, false, false)],
    ),
    (
        "openssl_pkey_export",
        &[
            (2, "passphrase", "?string", false, true, true, false, false),
            (3, "options", "?array", false, true, false, true, false),
        ],
    ),
    (
        "openssl_pkey_export_to_file",
        &[
            (
                1,
                "output_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (2, "passphrase", "?string", false, true, true, false, false),
            (3, "options", "?array", false, true, false, true, false),
        ],
    ),
    (
        "openssl_pkey_get_private",
        &[(1, "passphrase", "?string", false, true, true, false, false)],
    ),
    (
        "openssl_pkey_new",
        &[(0, "options", "?array", false, true, false, true, false)],
    ),
    (
        "openssl_private_decrypt",
        &[
            (0, "data", "string", false, false, true, false, false),
            (3, "padding", "int", false, false, false, false, false),
            (4, "digest_algo", "?string", false, true, true, false, false),
        ],
    ),
    (
        "openssl_private_encrypt",
        &[
            (0, "data", "string", false, false, true, false, false),
            (3, "padding", "int", false, false, false, false, false),
        ],
    ),
    (
        "openssl_public_decrypt",
        &[
            (0, "data", "string", false, false, true, false, false),
            (3, "padding", "int", false, false, false, false, false),
        ],
    ),
    (
        "openssl_public_encrypt",
        &[
            (0, "data", "string", false, false, true, false, false),
            (3, "padding", "int", false, false, false, false, false),
            (4, "digest_algo", "?string", false, true, true, false, false),
        ],
    ),
    (
        "openssl_random_pseudo_bytes",
        &[(0, "length", "int", false, false, false, false, false)],
    ),
    (
        "openssl_seal",
        &[
            (0, "data", "string", false, false, true, false, false),
            (3, "public_key", "array", false, false, false, true, false),
            (4, "cipher_algo", "string", false, false, true, false, false),
        ],
    ),
    (
        "openssl_sign",
        &[
            (0, "data", "string", false, false, true, false, false),
            (
                3,
                "algorithm",
                "string|int",
                false,
                false,
                true,
                false,
                false,
            ),
            (4, "padding", "int", false, false, false, false, false),
        ],
    ),
    (
        "openssl_spki_export",
        &[(0, "spki", "string", false, false, true, false, false)],
    ),
    (
        "openssl_spki_export_challenge",
        &[(0, "spki", "string", false, false, true, false, false)],
    ),
    (
        "openssl_spki_new",
        &[
            (1, "challenge", "string", false, false, true, false, false),
            (2, "digest_algo", "int", false, false, false, false, false),
        ],
    ),
    (
        "openssl_spki_verify",
        &[(0, "spki", "string", false, false, true, false, false)],
    ),
    (
        "openssl_verify",
        &[
            (0, "data", "string", false, false, true, false, false),
            (1, "signature", "string", false, false, true, false, false),
            (
                3,
                "algorithm",
                "string|int",
                false,
                false,
                true,
                false,
                false,
            ),
            (4, "padding", "int", false, false, false, false, false),
        ],
    ),
    (
        "openssl_x509_checkpurpose",
        &[
            (1, "purpose", "int", false, false, false, false, false),
            (2, "ca_info", "array", false, false, false, true, false),
            (
                3,
                "untrusted_certificates_file",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "openssl_x509_export",
        &[(2, "no_text", "bool", false, false, false, false, false)],
    ),
    (
        "openssl_x509_export_to_file",
        &[
            (
                1,
                "output_filename",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (2, "no_text", "bool", false, false, false, false, false),
        ],
    ),
    (
        "openssl_x509_fingerprint",
        &[
            (1, "digest_algo", "string", false, false, true, false, false),
            (2, "binary", "bool", false, false, false, false, false),
        ],
    ),
    (
        "openssl_x509_parse",
        &[(1, "short_names", "bool", false, false, false, false, false)],
    ),
    (
        "ord",
        &[(0, "character", "string", false, false, true, false, false)],
    ),
    (
        "output_add_rewrite_var",
        &[
            (0, "name", "string", false, false, true, false, false),
            (1, "value", "string", false, false, true, false, false),
        ],
    ),
    (
        "pack",
        &[(0, "format", "string", false, false, true, false, false)],
    ),
    (
        "parse_ini_file",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (
                1,
                "process_sections",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
            (2, "scanner_mode", "int", false, false, false, false, false),
        ],
    ),
    (
        "parse_ini_string",
        &[
            (0, "ini_string", "string", false, false, true, false, false),
            (
                1,
                "process_sections",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
            (2, "scanner_mode", "int", false, false, false, false, false),
        ],
    ),
    (
        "parse_str",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "parse_url",
        &[
            (0, "url", "string", false, false, true, false, false),
            (1, "component", "int", false, false, false, false, false),
        ],
    ),
    (
        "passthru",
        &[(0, "command", "string", false, false, true, false, false)],
    ),
    (
        "password_get_info",
        &[(0, "hash", "string", false, false, true, false, false)],
    ),
    (
        "password_hash",
        &[
            (0, "password", "string", false, false, true, false, false),
            (
                1,
                "algo",
                "string|int|null",
                false,
                true,
                true,
                false,
                false,
            ),
            (2, "options", "array", false, false, false, true, false),
        ],
    ),
    (
        "password_needs_rehash",
        &[
            (0, "hash", "string", false, false, true, false, false),
            (
                1,
                "algo",
                "string|int|null",
                false,
                true,
                true,
                false,
                false,
            ),
            (2, "options", "array", false, false, false, true, false),
        ],
    ),
    (
        "password_verify",
        &[
            (0, "password", "string", false, false, true, false, false),
            (1, "hash", "string", false, false, true, false, false),
        ],
    ),
    (
        "pathinfo",
        &[
            (0, "path", "string", false, false, true, false, false),
            (1, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "pcntl_wifcontinued",
        &[(0, "status", "int", false, false, false, false, false)],
    ),
    (
        "pfsockopen",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (1, "port", "int", false, false, false, false, false),
            (4, "timeout", "?float", false, true, false, false, false),
        ],
    ),
    (
        "php_strip_whitespace",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "php_uname",
        &[(0, "mode", "string", false, false, true, false, false)],
    ),
    (
        "phpcredits",
        &[(0, "flags", "int", false, false, false, false, false)],
    ),
    (
        "phpinfo",
        &[(0, "flags", "int", false, false, false, false, false)],
    ),
    (
        "phpversion",
        &[(0, "extension", "?string", false, true, true, false, false)],
    ),
    (
        "popen",
        &[
            (0, "command", "string", false, false, true, false, false),
            (1, "mode", "string", false, false, true, false, false),
        ],
    ),
    (
        "posix_access",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "posix_eaccess",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "posix_fpathconf",
        &[(1, "name", "int", false, false, false, false, false)],
    ),
    (
        "posix_getgrgid",
        &[(0, "group_id", "int", false, false, false, false, false)],
    ),
    (
        "posix_getgrnam",
        &[(0, "name", "string", false, false, true, false, false)],
    ),
    (
        "posix_getpgid",
        &[(0, "process_id", "int", false, false, false, false, false)],
    ),
    (
        "posix_getpwnam",
        &[(0, "username", "string", false, false, true, false, false)],
    ),
    (
        "posix_getpwuid",
        &[(0, "user_id", "int", false, false, false, false, false)],
    ),
    (
        "posix_getrlimit",
        &[(0, "resource", "?int", false, true, false, false, false)],
    ),
    (
        "posix_getsid",
        &[(0, "process_id", "int", false, false, false, false, false)],
    ),
    (
        "posix_initgroups",
        &[
            (0, "username", "string", false, false, true, false, false),
            (1, "group_id", "int", false, false, false, false, false),
        ],
    ),
    (
        "posix_kill",
        &[
            (0, "process_id", "int", false, false, false, false, false),
            (1, "signal", "int", false, false, false, false, false),
        ],
    ),
    (
        "posix_mkfifo",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "permissions", "int", false, false, false, false, false),
        ],
    ),
    (
        "posix_mknod",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "flags", "int", false, false, false, false, false),
            (2, "major", "int", false, false, false, false, false),
            (3, "minor", "int", false, false, false, false, false),
        ],
    ),
    (
        "posix_pathconf",
        &[
            (0, "path", "string", false, false, true, false, false),
            (1, "name", "int", false, false, false, false, false),
        ],
    ),
    (
        "posix_setegid",
        &[(0, "group_id", "int", false, false, false, false, false)],
    ),
    (
        "posix_seteuid",
        &[(0, "user_id", "int", false, false, false, false, false)],
    ),
    (
        "posix_setgid",
        &[(0, "group_id", "int", false, false, false, false, false)],
    ),
    (
        "posix_setpgid",
        &[
            (0, "process_id", "int", false, false, false, false, false),
            (
                1,
                "process_group_id",
                "int",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "posix_setrlimit",
        &[
            (0, "resource", "int", false, false, false, false, false),
            (1, "soft_limit", "int", false, false, false, false, false),
            (2, "hard_limit", "int", false, false, false, false, false),
        ],
    ),
    (
        "posix_setuid",
        &[(0, "user_id", "int", false, false, false, false, false)],
    ),
    (
        "posix_strerror",
        &[(0, "error_code", "int", false, false, false, false, false)],
    ),
    (
        "posix_sysconf",
        &[(0, "conf_id", "int", false, false, false, false, false)],
    ),
    (
        "preg_filter",
        &[
            (
                0,
                "pattern",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (
                1,
                "replacement",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (
                2,
                "subject",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (3, "limit", "int", false, false, false, false, false),
        ],
    ),
    (
        "preg_grep",
        &[
            (0, "pattern", "string", false, false, true, false, false),
            (1, "array", "array", false, false, false, true, false),
            (2, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "preg_match",
        &[
            (0, "pattern", "string", false, false, true, false, false),
            (1, "subject", "string", false, false, true, false, false),
            (3, "flags", "int", false, false, false, false, false),
            (4, "offset", "int", false, false, false, false, false),
        ],
    ),
    (
        "preg_match_all",
        &[
            (0, "pattern", "string", false, false, true, false, false),
            (1, "subject", "string", false, false, true, false, false),
            (3, "flags", "int", false, false, false, false, false),
            (4, "offset", "int", false, false, false, false, false),
        ],
    ),
    (
        "preg_quote",
        &[
            (0, "str", "string", false, false, true, false, false),
            (1, "delimiter", "?string", false, true, true, false, false),
        ],
    ),
    (
        "preg_replace",
        &[
            (
                0,
                "pattern",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (
                1,
                "replacement",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (
                2,
                "subject",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (3, "limit", "int", false, false, false, false, false),
        ],
    ),
    (
        "preg_replace_callback",
        &[
            (
                0,
                "pattern",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (
                2,
                "subject",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (3, "limit", "int", false, false, false, false, false),
            (5, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "preg_replace_callback_array",
        &[
            (0, "pattern", "array", false, false, false, true, false),
            (
                1,
                "subject",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (2, "limit", "int", false, false, false, false, false),
            (4, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "preg_split",
        &[
            (0, "pattern", "string", false, false, true, false, false),
            (1, "subject", "string", false, false, true, false, false),
            (2, "limit", "int", false, false, false, false, false),
            (3, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "print_r",
        &[(1, "return", "bool", false, false, false, false, false)],
    ),
    (
        "printf",
        &[(0, "format", "string", false, false, true, false, false)],
    ),
    (
        "proc_nice",
        &[(0, "priority", "int", false, false, false, false, false)],
    ),
    (
        "proc_open",
        &[
            (
                0,
                "command",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (
                1,
                "descriptor_spec",
                "array",
                false,
                false,
                false,
                true,
                false,
            ),
            (3, "cwd", "?string", false, true, true, false, false),
            (4, "env_vars", "?array", false, true, false, true, false),
            (5, "options", "?array", false, true, false, true, false),
        ],
    ),
    (
        "proc_terminate",
        &[(1, "signal", "int", false, false, false, false, false)],
    ),
    (
        "property_exists",
        &[(1, "property", "string", false, false, true, false, false)],
    ),
    (
        "putenv",
        &[(0, "assignment", "string", false, false, true, false, false)],
    ),
    (
        "quoted_printable_decode",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "quoted_printable_encode",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "quotemeta",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "rad2deg",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "range",
        &[(2, "step", "int|float", false, false, false, false, false)],
    ),
    (
        "rawurldecode",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "rawurlencode",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "readfile",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (
                1,
                "use_include_path",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "readgzfile",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (
                1,
                "use_include_path",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "readline",
        &[(0, "prompt", "?string", false, true, true, false, false)],
    ),
    (
        "readline_add_history",
        &[(0, "prompt", "string", false, false, true, false, false)],
    ),
    (
        "readline_callback_handler_install",
        &[(0, "prompt", "string", false, false, true, false, false)],
    ),
    (
        "readline_info",
        &[(0, "var_name", "?string", false, true, true, false, false)],
    ),
    (
        "readline_read_history",
        &[(0, "filename", "?string", false, true, true, false, false)],
    ),
    (
        "readline_write_history",
        &[(0, "filename", "?string", false, true, true, false, false)],
    ),
    (
        "readlink",
        &[(0, "path", "string", false, false, true, false, false)],
    ),
    (
        "realpath",
        &[(0, "path", "string", false, false, true, false, false)],
    ),
    (
        "rename",
        &[
            (0, "from", "string", false, false, true, false, false),
            (1, "to", "string", false, false, true, false, false),
        ],
    ),
    (
        "request_parse_body",
        &[(0, "options", "?array", false, true, false, true, false)],
    ),
    (
        "resourcebundle_create",
        &[
            (0, "locale", "?string", false, true, true, false, false),
            (1, "bundle", "?string", false, true, true, false, false),
            (2, "fallback", "bool", false, false, false, false, false),
        ],
    ),
    (
        "resourcebundle_get",
        &[
            (1, "index", "string|int", false, false, true, false, false),
            (2, "fallback", "bool", false, false, false, false, false),
        ],
    ),
    (
        "resourcebundle_locales",
        &[(0, "bundle", "string", false, false, true, false, false)],
    ),
    (
        "rmdir",
        &[(0, "directory", "string", false, false, true, false, false)],
    ),
    (
        "round",
        &[
            (0, "num", "int|float", false, false, false, false, false),
            (1, "precision", "int", false, false, false, false, false),
        ],
    ),
    (
        "rsort",
        &[
            (0, "array", "array", false, false, false, true, true),
            (1, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "rtrim",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "characters", "string", false, false, true, false, false),
        ],
    ),
    (
        "sapi_windows_cp_conv",
        &[
            (
                0,
                "in_codepage",
                "string|int",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                1,
                "out_codepage",
                "string|int",
                false,
                false,
                true,
                false,
                false,
            ),
            (2, "subject", "string", false, false, true, false, false),
        ],
    ),
    (
        "sapi_windows_cp_get",
        &[(0, "kind", "string", false, false, true, false, false)],
    ),
    (
        "sapi_windows_cp_set",
        &[(0, "codepage", "int", false, false, false, false, false)],
    ),
    (
        "sapi_windows_generate_ctrl_event",
        &[
            (0, "event", "int", false, false, false, false, false),
            (1, "pid", "int", false, false, false, false, false),
        ],
    ),
    (
        "sapi_windows_set_ctrl_handler",
        &[(1, "add", "bool", false, false, false, false, false)],
    ),
    (
        "sapi_windows_vt100_support",
        &[(1, "enable", "?bool", false, true, false, false, false)],
    ),
    (
        "scandir",
        &[
            (0, "directory", "string", false, false, true, false, false),
            (1, "sorting_order", "int", false, false, false, false, false),
        ],
    ),
    (
        "sem_acquire",
        &[(1, "non_blocking", "bool", false, false, false, false, false)],
    ),
    (
        "sem_get",
        &[
            (0, "key", "int", false, false, false, false, false),
            (1, "max_acquire", "int", false, false, false, false, false),
            (2, "permissions", "int", false, false, false, false, false),
            (3, "auto_release", "bool", false, false, false, false, false),
        ],
    ),
    (
        "session_cache_expire",
        &[(0, "value", "?int", false, true, false, false, false)],
    ),
    (
        "session_cache_limiter",
        &[(0, "value", "?string", false, true, true, false, false)],
    ),
    (
        "session_create_id",
        &[(0, "prefix", "string", false, false, true, false, false)],
    ),
    (
        "session_decode",
        &[(0, "data", "string", false, false, true, false, false)],
    ),
    (
        "session_id",
        &[(0, "id", "?string", false, true, true, false, false)],
    ),
    (
        "session_module_name",
        &[(0, "module", "?string", false, true, true, false, false)],
    ),
    (
        "session_name",
        &[(0, "name", "?string", false, true, true, false, false)],
    ),
    (
        "session_regenerate_id",
        &[(
            0,
            "delete_old_session",
            "bool",
            false,
            false,
            false,
            false,
            false,
        )],
    ),
    (
        "session_save_path",
        &[(0, "path", "?string", false, true, true, false, false)],
    ),
    (
        "session_set_cookie_params",
        &[
            (
                0,
                "lifetime_or_options",
                "array|int",
                false,
                false,
                false,
                true,
                false,
            ),
            (1, "path", "?string", false, true, true, false, false),
            (2, "domain", "?string", false, true, true, false, false),
            (3, "secure", "?bool", false, true, false, false, false),
            (4, "httponly", "?bool", false, true, false, false, false),
        ],
    ),
    (
        "session_start",
        &[(0, "options", "array", false, false, false, true, false)],
    ),
    (
        "set_error_handler",
        &[(1, "error_levels", "int", false, false, false, false, false)],
    ),
    (
        "set_file_buffer",
        &[(1, "size", "int", false, false, false, false, false)],
    ),
    (
        "set_include_path",
        &[(
            0,
            "include_path",
            "string",
            false,
            false,
            true,
            false,
            false,
        )],
    ),
    (
        "set_time_limit",
        &[(0, "seconds", "int", false, false, false, false, false)],
    ),
    (
        "setcookie",
        &[
            (0, "name", "string", false, false, true, false, false),
            (1, "value", "string", false, false, true, false, false),
            (
                2,
                "expires_or_options",
                "array|int",
                false,
                false,
                false,
                true,
                false,
            ),
            (3, "path", "string", false, false, true, false, false),
            (4, "domain", "string", false, false, true, false, false),
            (5, "secure", "bool", false, false, false, false, false),
            (6, "httponly", "bool", false, false, false, false, false),
        ],
    ),
    (
        "setlocale",
        &[(0, "category", "int", false, false, false, false, false)],
    ),
    (
        "setrawcookie",
        &[
            (0, "name", "string", false, false, true, false, false),
            (1, "value", "string", false, false, true, false, false),
            (
                2,
                "expires_or_options",
                "array|int",
                false,
                false,
                false,
                true,
                false,
            ),
            (3, "path", "string", false, false, true, false, false),
            (4, "domain", "string", false, false, true, false, false),
            (5, "secure", "bool", false, false, false, false, false),
            (6, "httponly", "bool", false, false, false, false, false),
        ],
    ),
    (
        "settype",
        &[(1, "type", "string", false, false, true, false, false)],
    ),
    (
        "sha1",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "binary", "bool", false, false, false, false, false),
        ],
    ),
    (
        "sha1_file",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "binary", "bool", false, false, false, false, false),
        ],
    ),
    (
        "shell_exec",
        &[(0, "command", "string", false, false, true, false, false)],
    ),
    (
        "shm_attach",
        &[
            (0, "key", "int", false, false, false, false, false),
            (1, "size", "?int", false, true, false, false, false),
            (2, "permissions", "int", false, false, false, false, false),
        ],
    ),
    (
        "shm_get_var",
        &[(1, "key", "int", false, false, false, false, false)],
    ),
    (
        "shm_has_var",
        &[(1, "key", "int", false, false, false, false, false)],
    ),
    (
        "shm_put_var",
        &[(1, "key", "int", false, false, false, false, false)],
    ),
    (
        "shm_remove_var",
        &[(1, "key", "int", false, false, false, false, false)],
    ),
    (
        "shmop_open",
        &[
            (0, "key", "int", false, false, false, false, false),
            (1, "mode", "string", false, false, true, false, false),
            (2, "permissions", "int", false, false, false, false, false),
            (3, "size", "int", false, false, false, false, false),
        ],
    ),
    (
        "shmop_read",
        &[
            (1, "offset", "int", false, false, false, false, false),
            (2, "size", "int", false, false, false, false, false),
        ],
    ),
    (
        "shmop_write",
        &[
            (1, "data", "string", false, false, true, false, false),
            (2, "offset", "int", false, false, false, false, false),
        ],
    ),
    (
        "show_source",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "return", "bool", false, false, false, false, false),
        ],
    ),
    (
        "shuffle",
        &[(0, "array", "array", false, false, false, true, true)],
    ),
    (
        "similar_text",
        &[
            (0, "string1", "string", false, false, true, false, false),
            (1, "string2", "string", false, false, true, false, false),
        ],
    ),
    (
        "simplexml_import_dom",
        &[(1, "class_name", "?string", false, true, true, false, false)],
    ),
    (
        "simplexml_load_file",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "class_name", "?string", false, true, true, false, false),
            (2, "options", "int", false, false, false, false, false),
            (
                3,
                "namespace_or_prefix",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (4, "is_prefix", "bool", false, false, false, false, false),
        ],
    ),
    (
        "simplexml_load_string",
        &[
            (0, "data", "string", false, false, true, false, false),
            (1, "class_name", "?string", false, true, true, false, false),
            (2, "options", "int", false, false, false, false, false),
            (
                3,
                "namespace_or_prefix",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (4, "is_prefix", "bool", false, false, false, false, false),
        ],
    ),
    (
        "sin",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "sinh",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "sizeof",
        &[(1, "mode", "int", false, false, false, false, false)],
    ),
    (
        "sleep",
        &[(0, "seconds", "int", false, false, false, false, false)],
    ),
    (
        "snmp2_get",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (1, "community", "string", false, false, true, false, false),
            (
                2,
                "object_id",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (3, "timeout", "int", false, false, false, false, false),
            (4, "retries", "int", false, false, false, false, false),
        ],
    ),
    (
        "snmp2_getnext",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (1, "community", "string", false, false, true, false, false),
            (
                2,
                "object_id",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (3, "timeout", "int", false, false, false, false, false),
            (4, "retries", "int", false, false, false, false, false),
        ],
    ),
    (
        "snmp2_real_walk",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (1, "community", "string", false, false, true, false, false),
            (
                2,
                "object_id",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (3, "timeout", "int", false, false, false, false, false),
            (4, "retries", "int", false, false, false, false, false),
        ],
    ),
    (
        "snmp2_set",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (1, "community", "string", false, false, true, false, false),
            (
                2,
                "object_id",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (3, "type", "array|string", false, false, true, true, false),
            (4, "value", "array|string", false, false, true, true, false),
            (5, "timeout", "int", false, false, false, false, false),
            (6, "retries", "int", false, false, false, false, false),
        ],
    ),
    (
        "snmp2_walk",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (1, "community", "string", false, false, true, false, false),
            (
                2,
                "object_id",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (3, "timeout", "int", false, false, false, false, false),
            (4, "retries", "int", false, false, false, false, false),
        ],
    ),
    (
        "snmp3_get",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (
                1,
                "security_name",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                2,
                "security_level",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                3,
                "auth_protocol",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                4,
                "auth_passphrase",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                5,
                "privacy_protocol",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                6,
                "privacy_passphrase",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                7,
                "object_id",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (8, "timeout", "int", false, false, false, false, false),
            (9, "retries", "int", false, false, false, false, false),
        ],
    ),
    (
        "snmp3_getnext",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (
                1,
                "security_name",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                2,
                "security_level",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                3,
                "auth_protocol",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                4,
                "auth_passphrase",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                5,
                "privacy_protocol",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                6,
                "privacy_passphrase",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                7,
                "object_id",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (8, "timeout", "int", false, false, false, false, false),
            (9, "retries", "int", false, false, false, false, false),
        ],
    ),
    (
        "snmp3_real_walk",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (
                1,
                "security_name",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                2,
                "security_level",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                3,
                "auth_protocol",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                4,
                "auth_passphrase",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                5,
                "privacy_protocol",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                6,
                "privacy_passphrase",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                7,
                "object_id",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (8, "timeout", "int", false, false, false, false, false),
            (9, "retries", "int", false, false, false, false, false),
        ],
    ),
    (
        "snmp3_set",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (
                1,
                "security_name",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                2,
                "security_level",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                3,
                "auth_protocol",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                4,
                "auth_passphrase",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                5,
                "privacy_protocol",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                6,
                "privacy_passphrase",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                7,
                "object_id",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (8, "type", "array|string", false, false, true, true, false),
            (9, "value", "array|string", false, false, true, true, false),
            (10, "timeout", "int", false, false, false, false, false),
            (11, "retries", "int", false, false, false, false, false),
        ],
    ),
    (
        "snmp3_walk",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (
                1,
                "security_name",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                2,
                "security_level",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                3,
                "auth_protocol",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                4,
                "auth_passphrase",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                5,
                "privacy_protocol",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                6,
                "privacy_passphrase",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (
                7,
                "object_id",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (8, "timeout", "int", false, false, false, false, false),
            (9, "retries", "int", false, false, false, false, false),
        ],
    ),
    (
        "snmp_read_mib",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "snmp_set_enum_print",
        &[(0, "enable", "bool", false, false, false, false, false)],
    ),
    (
        "snmp_set_oid_numeric_print",
        &[(0, "format", "int", false, false, false, false, false)],
    ),
    (
        "snmp_set_oid_output_format",
        &[(0, "format", "int", false, false, false, false, false)],
    ),
    (
        "snmp_set_quick_print",
        &[(0, "enable", "bool", false, false, false, false, false)],
    ),
    (
        "snmp_set_valueretrieval",
        &[(0, "method", "int", false, false, false, false, false)],
    ),
    (
        "snmpget",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (1, "community", "string", false, false, true, false, false),
            (
                2,
                "object_id",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (3, "timeout", "int", false, false, false, false, false),
            (4, "retries", "int", false, false, false, false, false),
        ],
    ),
    (
        "snmpgetnext",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (1, "community", "string", false, false, true, false, false),
            (
                2,
                "object_id",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (3, "timeout", "int", false, false, false, false, false),
            (4, "retries", "int", false, false, false, false, false),
        ],
    ),
    (
        "snmprealwalk",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (1, "community", "string", false, false, true, false, false),
            (
                2,
                "object_id",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (3, "timeout", "int", false, false, false, false, false),
            (4, "retries", "int", false, false, false, false, false),
        ],
    ),
    (
        "snmpset",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (1, "community", "string", false, false, true, false, false),
            (
                2,
                "object_id",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (3, "type", "array|string", false, false, true, true, false),
            (4, "value", "array|string", false, false, true, true, false),
            (5, "timeout", "int", false, false, false, false, false),
            (6, "retries", "int", false, false, false, false, false),
        ],
    ),
    (
        "snmpwalk",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (1, "community", "string", false, false, true, false, false),
            (
                2,
                "object_id",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (3, "timeout", "int", false, false, false, false, false),
            (4, "retries", "int", false, false, false, false, false),
        ],
    ),
    (
        "snmpwalkoid",
        &[
            (0, "hostname", "string", false, false, true, false, false),
            (1, "community", "string", false, false, true, false, false),
            (
                2,
                "object_id",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (3, "timeout", "int", false, false, false, false, false),
            (4, "retries", "int", false, false, false, false, false),
        ],
    ),
    (
        "socket_addrinfo_lookup",
        &[
            (0, "host", "string", false, false, true, false, false),
            (1, "service", "?string", false, true, true, false, false),
            (2, "hints", "array", false, false, false, true, false),
        ],
    ),
    (
        "socket_bind",
        &[
            (1, "address", "string", false, false, true, false, false),
            (2, "port", "int", false, false, false, false, false),
        ],
    ),
    (
        "socket_cmsg_space",
        &[
            (0, "level", "int", false, false, false, false, false),
            (1, "type", "int", false, false, false, false, false),
            (2, "num", "int", false, false, false, false, false),
        ],
    ),
    (
        "socket_connect",
        &[
            (1, "address", "string", false, false, true, false, false),
            (2, "port", "?int", false, true, false, false, false),
        ],
    ),
    (
        "socket_create",
        &[
            (0, "domain", "int", false, false, false, false, false),
            (1, "type", "int", false, false, false, false, false),
            (2, "protocol", "int", false, false, false, false, false),
        ],
    ),
    (
        "socket_create_listen",
        &[
            (0, "port", "int", false, false, false, false, false),
            (1, "backlog", "int", false, false, false, false, false),
        ],
    ),
    (
        "socket_create_pair",
        &[
            (0, "domain", "int", false, false, false, false, false),
            (1, "type", "int", false, false, false, false, false),
            (2, "protocol", "int", false, false, false, false, false),
        ],
    ),
    (
        "socket_get_option",
        &[
            (1, "level", "int", false, false, false, false, false),
            (2, "option", "int", false, false, false, false, false),
        ],
    ),
    (
        "socket_getopt",
        &[
            (1, "level", "int", false, false, false, false, false),
            (2, "option", "int", false, false, false, false, false),
        ],
    ),
    (
        "socket_listen",
        &[(1, "backlog", "int", false, false, false, false, false)],
    ),
    (
        "socket_read",
        &[
            (1, "length", "int", false, false, false, false, false),
            (2, "mode", "int", false, false, false, false, false),
        ],
    ),
    (
        "socket_recv",
        &[
            (2, "length", "int", false, false, false, false, false),
            (3, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "socket_recvfrom",
        &[
            (2, "length", "int", false, false, false, false, false),
            (3, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "socket_recvmsg",
        &[
            (1, "message", "array", false, false, false, true, true),
            (2, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "socket_select",
        &[
            (3, "seconds", "?int", false, true, false, false, false),
            (4, "microseconds", "int", false, false, false, false, false),
        ],
    ),
    (
        "socket_send",
        &[
            (1, "data", "string", false, false, true, false, false),
            (2, "length", "int", false, false, false, false, false),
            (3, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "socket_sendmsg",
        &[
            (1, "message", "array", false, false, false, true, false),
            (2, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "socket_sendto",
        &[
            (1, "data", "string", false, false, true, false, false),
            (2, "length", "int", false, false, false, false, false),
            (3, "flags", "int", false, false, false, false, false),
            (4, "address", "string", false, false, true, false, false),
            (5, "port", "?int", false, true, false, false, false),
        ],
    ),
    (
        "socket_set_blocking",
        &[(1, "enable", "bool", false, false, false, false, false)],
    ),
    (
        "socket_set_option",
        &[
            (1, "level", "int", false, false, false, false, false),
            (2, "option", "int", false, false, false, false, false),
        ],
    ),
    (
        "socket_set_timeout",
        &[
            (1, "seconds", "int", false, false, false, false, false),
            (2, "microseconds", "int", false, false, false, false, false),
        ],
    ),
    (
        "socket_setopt",
        &[
            (1, "level", "int", false, false, false, false, false),
            (2, "option", "int", false, false, false, false, false),
        ],
    ),
    (
        "socket_shutdown",
        &[(1, "mode", "int", false, false, false, false, false)],
    ),
    (
        "socket_strerror",
        &[(0, "error_code", "int", false, false, false, false, false)],
    ),
    (
        "socket_write",
        &[
            (1, "data", "string", false, false, true, false, false),
            (2, "length", "?int", false, true, false, false, false),
        ],
    ),
    (
        "socket_wsaprotocol_info_export",
        &[(1, "process_id", "int", false, false, false, false, false)],
    ),
    (
        "socket_wsaprotocol_info_import",
        &[(0, "info_id", "string", false, false, true, false, false)],
    ),
    (
        "socket_wsaprotocol_info_release",
        &[(0, "info_id", "string", false, false, true, false, false)],
    ),
    (
        "sodium_add",
        &[(1, "string2", "string", false, false, true, false, false)],
    ),
    (
        "sodium_base642bin",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "id", "int", false, false, false, false, false),
            (2, "ignore", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_bin2base64",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "id", "int", false, false, false, false, false),
        ],
    ),
    (
        "sodium_bin2hex",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "sodium_compare",
        &[
            (0, "string1", "string", false, false, true, false, false),
            (1, "string2", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_aead_aegis128l_decrypt",
        &[
            (0, "ciphertext", "string", false, false, true, false, false),
            (
                1,
                "additional_data",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (2, "nonce", "string", false, false, true, false, false),
            (3, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_aead_aegis128l_encrypt",
        &[
            (0, "message", "string", false, false, true, false, false),
            (
                1,
                "additional_data",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (2, "nonce", "string", false, false, true, false, false),
            (3, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_aead_aegis256_decrypt",
        &[
            (0, "ciphertext", "string", false, false, true, false, false),
            (
                1,
                "additional_data",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (2, "nonce", "string", false, false, true, false, false),
            (3, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_aead_aegis256_encrypt",
        &[
            (0, "message", "string", false, false, true, false, false),
            (
                1,
                "additional_data",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (2, "nonce", "string", false, false, true, false, false),
            (3, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_aead_aes256gcm_decrypt",
        &[
            (0, "ciphertext", "string", false, false, true, false, false),
            (
                1,
                "additional_data",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (2, "nonce", "string", false, false, true, false, false),
            (3, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_aead_aes256gcm_encrypt",
        &[
            (0, "message", "string", false, false, true, false, false),
            (
                1,
                "additional_data",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (2, "nonce", "string", false, false, true, false, false),
            (3, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_aead_chacha20poly1305_decrypt",
        &[
            (0, "ciphertext", "string", false, false, true, false, false),
            (
                1,
                "additional_data",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (2, "nonce", "string", false, false, true, false, false),
            (3, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_aead_chacha20poly1305_encrypt",
        &[
            (0, "message", "string", false, false, true, false, false),
            (
                1,
                "additional_data",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (2, "nonce", "string", false, false, true, false, false),
            (3, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_aead_chacha20poly1305_ietf_decrypt",
        &[
            (0, "ciphertext", "string", false, false, true, false, false),
            (
                1,
                "additional_data",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (2, "nonce", "string", false, false, true, false, false),
            (3, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_aead_chacha20poly1305_ietf_encrypt",
        &[
            (0, "message", "string", false, false, true, false, false),
            (
                1,
                "additional_data",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (2, "nonce", "string", false, false, true, false, false),
            (3, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_aead_xchacha20poly1305_ietf_decrypt",
        &[
            (0, "ciphertext", "string", false, false, true, false, false),
            (
                1,
                "additional_data",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (2, "nonce", "string", false, false, true, false, false),
            (3, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_aead_xchacha20poly1305_ietf_encrypt",
        &[
            (0, "message", "string", false, false, true, false, false),
            (
                1,
                "additional_data",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (2, "nonce", "string", false, false, true, false, false),
            (3, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_auth",
        &[
            (0, "message", "string", false, false, true, false, false),
            (1, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_auth_verify",
        &[
            (0, "mac", "string", false, false, true, false, false),
            (1, "message", "string", false, false, true, false, false),
            (2, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_box",
        &[
            (0, "message", "string", false, false, true, false, false),
            (1, "nonce", "string", false, false, true, false, false),
            (2, "key_pair", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_box_keypair_from_secretkey_and_publickey",
        &[
            (0, "secret_key", "string", false, false, true, false, false),
            (1, "public_key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_box_open",
        &[
            (0, "ciphertext", "string", false, false, true, false, false),
            (1, "nonce", "string", false, false, true, false, false),
            (2, "key_pair", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_box_publickey",
        &[(0, "key_pair", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_box_publickey_from_secretkey",
        &[(0, "secret_key", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_box_seal",
        &[
            (0, "message", "string", false, false, true, false, false),
            (1, "public_key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_box_seal_open",
        &[
            (0, "ciphertext", "string", false, false, true, false, false),
            (1, "key_pair", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_box_secretkey",
        &[(0, "key_pair", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_box_seed_keypair",
        &[(0, "seed", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_core_ristretto255_add",
        &[
            (0, "p", "string", false, false, true, false, false),
            (1, "q", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_core_ristretto255_from_hash",
        &[(0, "s", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_core_ristretto255_is_valid_point",
        &[(0, "s", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_core_ristretto255_scalar_add",
        &[
            (0, "x", "string", false, false, true, false, false),
            (1, "y", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_core_ristretto255_scalar_complement",
        &[(0, "s", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_core_ristretto255_scalar_invert",
        &[(0, "s", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_core_ristretto255_scalar_mul",
        &[
            (0, "x", "string", false, false, true, false, false),
            (1, "y", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_core_ristretto255_scalar_negate",
        &[(0, "s", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_core_ristretto255_scalar_reduce",
        &[(0, "s", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_core_ristretto255_scalar_sub",
        &[
            (0, "x", "string", false, false, true, false, false),
            (1, "y", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_core_ristretto255_sub",
        &[
            (0, "p", "string", false, false, true, false, false),
            (1, "q", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_generichash",
        &[
            (0, "message", "string", false, false, true, false, false),
            (1, "key", "string", false, false, true, false, false),
            (2, "length", "int", false, false, false, false, false),
        ],
    ),
    (
        "sodium_crypto_generichash_final",
        &[(1, "length", "int", false, false, false, false, false)],
    ),
    (
        "sodium_crypto_generichash_init",
        &[
            (0, "key", "string", false, false, true, false, false),
            (1, "length", "int", false, false, false, false, false),
        ],
    ),
    (
        "sodium_crypto_generichash_update",
        &[(1, "message", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_kdf_derive_from_key",
        &[
            (0, "subkey_length", "int", false, false, false, false, false),
            (1, "subkey_id", "int", false, false, false, false, false),
            (2, "context", "string", false, false, true, false, false),
            (3, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_kx_client_session_keys",
        &[
            (
                0,
                "client_key_pair",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (1, "server_key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_kx_publickey",
        &[(0, "key_pair", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_kx_secretkey",
        &[(0, "key_pair", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_kx_seed_keypair",
        &[(0, "seed", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_kx_server_session_keys",
        &[
            (
                0,
                "server_key_pair",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (1, "client_key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_pwhash",
        &[
            (0, "length", "int", false, false, false, false, false),
            (1, "password", "string", false, false, true, false, false),
            (2, "salt", "string", false, false, true, false, false),
            (3, "opslimit", "int", false, false, false, false, false),
            (4, "memlimit", "int", false, false, false, false, false),
            (5, "algo", "int", false, false, false, false, false),
        ],
    ),
    (
        "sodium_crypto_pwhash_scryptsalsa208sha256",
        &[
            (0, "length", "int", false, false, false, false, false),
            (1, "password", "string", false, false, true, false, false),
            (2, "salt", "string", false, false, true, false, false),
            (3, "opslimit", "int", false, false, false, false, false),
            (4, "memlimit", "int", false, false, false, false, false),
        ],
    ),
    (
        "sodium_crypto_pwhash_scryptsalsa208sha256_str",
        &[
            (0, "password", "string", false, false, true, false, false),
            (1, "opslimit", "int", false, false, false, false, false),
            (2, "memlimit", "int", false, false, false, false, false),
        ],
    ),
    (
        "sodium_crypto_pwhash_scryptsalsa208sha256_str_verify",
        &[
            (0, "hash", "string", false, false, true, false, false),
            (1, "password", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_pwhash_str",
        &[
            (0, "password", "string", false, false, true, false, false),
            (1, "opslimit", "int", false, false, false, false, false),
            (2, "memlimit", "int", false, false, false, false, false),
        ],
    ),
    (
        "sodium_crypto_pwhash_str_needs_rehash",
        &[
            (0, "password", "string", false, false, true, false, false),
            (1, "opslimit", "int", false, false, false, false, false),
            (2, "memlimit", "int", false, false, false, false, false),
        ],
    ),
    (
        "sodium_crypto_pwhash_str_verify",
        &[
            (0, "hash", "string", false, false, true, false, false),
            (1, "password", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_scalarmult",
        &[
            (0, "n", "string", false, false, true, false, false),
            (1, "p", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_scalarmult_base",
        &[(0, "secret_key", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_scalarmult_ristretto255",
        &[
            (0, "n", "string", false, false, true, false, false),
            (1, "p", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_scalarmult_ristretto255_base",
        &[(0, "n", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_secretbox",
        &[
            (0, "message", "string", false, false, true, false, false),
            (1, "nonce", "string", false, false, true, false, false),
            (2, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_secretbox_open",
        &[
            (0, "ciphertext", "string", false, false, true, false, false),
            (1, "nonce", "string", false, false, true, false, false),
            (2, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_secretstream_xchacha20poly1305_init_pull",
        &[
            (0, "header", "string", false, false, true, false, false),
            (1, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_secretstream_xchacha20poly1305_init_push",
        &[(0, "key", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_secretstream_xchacha20poly1305_pull",
        &[
            (1, "ciphertext", "string", false, false, true, false, false),
            (
                2,
                "additional_data",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "sodium_crypto_secretstream_xchacha20poly1305_push",
        &[
            (1, "message", "string", false, false, true, false, false),
            (
                2,
                "additional_data",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (3, "tag", "int", false, false, false, false, false),
        ],
    ),
    (
        "sodium_crypto_shorthash",
        &[
            (0, "message", "string", false, false, true, false, false),
            (1, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_sign",
        &[
            (0, "message", "string", false, false, true, false, false),
            (1, "secret_key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_sign_detached",
        &[
            (0, "message", "string", false, false, true, false, false),
            (1, "secret_key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_sign_ed25519_pk_to_curve25519",
        &[(0, "public_key", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_sign_ed25519_sk_to_curve25519",
        &[(0, "secret_key", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_sign_keypair_from_secretkey_and_publickey",
        &[
            (0, "secret_key", "string", false, false, true, false, false),
            (1, "public_key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_sign_open",
        &[
            (
                0,
                "signed_message",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (1, "public_key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_sign_publickey",
        &[(0, "key_pair", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_sign_publickey_from_secretkey",
        &[(0, "secret_key", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_sign_secretkey",
        &[(0, "key_pair", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_sign_seed_keypair",
        &[(0, "seed", "string", false, false, true, false, false)],
    ),
    (
        "sodium_crypto_sign_verify_detached",
        &[
            (0, "signature", "string", false, false, true, false, false),
            (1, "message", "string", false, false, true, false, false),
            (2, "public_key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_stream",
        &[
            (0, "length", "int", false, false, false, false, false),
            (1, "nonce", "string", false, false, true, false, false),
            (2, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_stream_xchacha20",
        &[
            (0, "length", "int", false, false, false, false, false),
            (1, "nonce", "string", false, false, true, false, false),
            (2, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_stream_xchacha20_xor",
        &[
            (0, "message", "string", false, false, true, false, false),
            (1, "nonce", "string", false, false, true, false, false),
            (2, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_stream_xchacha20_xor_ic",
        &[
            (0, "message", "string", false, false, true, false, false),
            (1, "nonce", "string", false, false, true, false, false),
            (2, "counter", "int", false, false, false, false, false),
            (3, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_crypto_stream_xor",
        &[
            (0, "message", "string", false, false, true, false, false),
            (1, "nonce", "string", false, false, true, false, false),
            (2, "key", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_hex2bin",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "ignore", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_memcmp",
        &[
            (0, "string1", "string", false, false, true, false, false),
            (1, "string2", "string", false, false, true, false, false),
        ],
    ),
    (
        "sodium_pad",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "block_size", "int", false, false, false, false, false),
        ],
    ),
    (
        "sodium_unpad",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "block_size", "int", false, false, false, false, false),
        ],
    ),
    (
        "sort",
        &[
            (0, "array", "array", false, false, false, true, true),
            (1, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "soundex",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "spl_autoload",
        &[
            (0, "class", "string", false, false, true, false, false),
            (
                1,
                "file_extensions",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "spl_autoload_call",
        &[(0, "class", "string", false, false, true, false, false)],
    ),
    (
        "spl_autoload_extensions",
        &[(
            0,
            "file_extensions",
            "?string",
            false,
            true,
            true,
            false,
            false,
        )],
    ),
    (
        "spl_autoload_register",
        &[
            (1, "throw", "bool", false, false, false, false, false),
            (2, "prepend", "bool", false, false, false, false, false),
        ],
    ),
    (
        "sprintf",
        &[(0, "format", "string", false, false, true, false, false)],
    ),
    (
        "sqrt",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "sscanf",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "format", "string", false, false, true, false, false),
        ],
    ),
    (
        "stat",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "str_contains",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
        ],
    ),
    (
        "str_decrement",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "str_ends_with",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
        ],
    ),
    (
        "str_getcsv",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "separator", "string", false, false, true, false, false),
            (2, "enclosure", "string", false, false, true, false, false),
        ],
    ),
    (
        "str_increment",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "str_ireplace",
        &[
            (0, "search", "array|string", false, false, true, true, false),
            (
                1,
                "replace",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (
                2,
                "subject",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
        ],
    ),
    (
        "str_pad",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "length", "int", false, false, false, false, false),
            (2, "pad_string", "string", false, false, true, false, false),
            (3, "pad_type", "int", false, false, false, false, false),
        ],
    ),
    (
        "str_repeat",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "times", "int", false, false, false, false, false),
        ],
    ),
    (
        "str_replace",
        &[
            (0, "search", "array|string", false, false, true, true, false),
            (
                1,
                "replace",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (
                2,
                "subject",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
        ],
    ),
    (
        "str_rot13",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "str_shuffle",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "str_split",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "length", "int", false, false, false, false, false),
        ],
    ),
    (
        "str_starts_with",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
        ],
    ),
    (
        "str_word_count",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "format", "int", false, false, false, false, false),
            (2, "characters", "?string", false, true, true, false, false),
        ],
    ),
    (
        "strcasecmp",
        &[
            (0, "string1", "string", false, false, true, false, false),
            (1, "string2", "string", false, false, true, false, false),
        ],
    ),
    (
        "strchr",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (
                2,
                "before_needle",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "strcmp",
        &[
            (0, "string1", "string", false, false, true, false, false),
            (1, "string2", "string", false, false, true, false, false),
        ],
    ),
    (
        "strcoll",
        &[
            (0, "string1", "string", false, false, true, false, false),
            (1, "string2", "string", false, false, true, false, false),
        ],
    ),
    (
        "strcspn",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "characters", "string", false, false, true, false, false),
            (2, "offset", "int", false, false, false, false, false),
            (3, "length", "?int", false, true, false, false, false),
        ],
    ),
    (
        "stream_bucket_new",
        &[(1, "buffer", "string", false, false, true, false, false)],
    ),
    (
        "stream_context_create",
        &[
            (0, "options", "?array", false, true, false, true, false),
            (1, "params", "?array", false, true, false, true, false),
        ],
    ),
    (
        "stream_context_get_default",
        &[(0, "options", "?array", false, true, false, true, false)],
    ),
    (
        "stream_context_set_default",
        &[(0, "options", "array", false, false, false, true, false)],
    ),
    (
        "stream_context_set_option",
        &[
            (
                1,
                "wrapper_or_options",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (2, "option_name", "?string", false, true, true, false, false),
        ],
    ),
    (
        "stream_context_set_options",
        &[(1, "options", "array", false, false, false, true, false)],
    ),
    (
        "stream_context_set_params",
        &[(1, "params", "array", false, false, false, true, false)],
    ),
    (
        "stream_copy_to_stream",
        &[
            (2, "length", "?int", false, true, false, false, false),
            (3, "offset", "int", false, false, false, false, false),
        ],
    ),
    (
        "stream_filter_append",
        &[
            (1, "filter_name", "string", false, false, true, false, false),
            (2, "mode", "int", false, false, false, false, false),
        ],
    ),
    (
        "stream_filter_prepend",
        &[
            (1, "filter_name", "string", false, false, true, false, false),
            (2, "mode", "int", false, false, false, false, false),
        ],
    ),
    (
        "stream_filter_register",
        &[
            (0, "filter_name", "string", false, false, true, false, false),
            (1, "class", "string", false, false, true, false, false),
        ],
    ),
    (
        "stream_get_contents",
        &[
            (1, "length", "?int", false, true, false, false, false),
            (2, "offset", "int", false, false, false, false, false),
        ],
    ),
    (
        "stream_get_line",
        &[
            (1, "length", "int", false, false, false, false, false),
            (2, "ending", "string", false, false, true, false, false),
        ],
    ),
    (
        "stream_register_wrapper",
        &[
            (0, "protocol", "string", false, false, true, false, false),
            (1, "class", "string", false, false, true, false, false),
            (2, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "stream_resolve_include_path",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "stream_select",
        &[
            (3, "seconds", "?int", false, true, false, false, false),
            (4, "microseconds", "?int", false, true, false, false, false),
        ],
    ),
    (
        "stream_set_blocking",
        &[(1, "enable", "bool", false, false, false, false, false)],
    ),
    (
        "stream_set_chunk_size",
        &[(1, "size", "int", false, false, false, false, false)],
    ),
    (
        "stream_set_read_buffer",
        &[(1, "size", "int", false, false, false, false, false)],
    ),
    (
        "stream_set_timeout",
        &[
            (1, "seconds", "int", false, false, false, false, false),
            (2, "microseconds", "int", false, false, false, false, false),
        ],
    ),
    (
        "stream_set_write_buffer",
        &[(1, "size", "int", false, false, false, false, false)],
    ),
    (
        "stream_socket_accept",
        &[(1, "timeout", "?float", false, true, false, false, false)],
    ),
    (
        "stream_socket_client",
        &[
            (0, "address", "string", false, false, true, false, false),
            (3, "timeout", "?float", false, true, false, false, false),
            (4, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "stream_socket_enable_crypto",
        &[
            (1, "enable", "bool", false, false, false, false, false),
            (2, "crypto_method", "?int", false, true, false, false, false),
        ],
    ),
    (
        "stream_socket_get_name",
        &[(1, "remote", "bool", false, false, false, false, false)],
    ),
    (
        "stream_socket_pair",
        &[
            (0, "domain", "int", false, false, false, false, false),
            (1, "type", "int", false, false, false, false, false),
            (2, "protocol", "int", false, false, false, false, false),
        ],
    ),
    (
        "stream_socket_recvfrom",
        &[
            (1, "length", "int", false, false, false, false, false),
            (2, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "stream_socket_sendto",
        &[
            (1, "data", "string", false, false, true, false, false),
            (2, "flags", "int", false, false, false, false, false),
            (3, "address", "string", false, false, true, false, false),
        ],
    ),
    (
        "stream_socket_server",
        &[
            (0, "address", "string", false, false, true, false, false),
            (3, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "stream_socket_shutdown",
        &[(1, "mode", "int", false, false, false, false, false)],
    ),
    (
        "stream_wrapper_register",
        &[
            (0, "protocol", "string", false, false, true, false, false),
            (1, "class", "string", false, false, true, false, false),
            (2, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "stream_wrapper_restore",
        &[(0, "protocol", "string", false, false, true, false, false)],
    ),
    (
        "stream_wrapper_unregister",
        &[(0, "protocol", "string", false, false, true, false, false)],
    ),
    (
        "strftime",
        &[
            (0, "format", "string", false, false, true, false, false),
            (1, "timestamp", "?int", false, true, false, false, false),
        ],
    ),
    (
        "strip_tags",
        &[
            (0, "string", "string", false, false, true, false, false),
            (
                1,
                "allowed_tags",
                "array|string|null",
                false,
                true,
                true,
                true,
                false,
            ),
        ],
    ),
    (
        "stripcslashes",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "stripos",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (2, "offset", "int", false, false, false, false, false),
        ],
    ),
    (
        "stripslashes",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "stristr",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (
                2,
                "before_needle",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "strlen",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "strnatcasecmp",
        &[
            (0, "string1", "string", false, false, true, false, false),
            (1, "string2", "string", false, false, true, false, false),
        ],
    ),
    (
        "strnatcmp",
        &[
            (0, "string1", "string", false, false, true, false, false),
            (1, "string2", "string", false, false, true, false, false),
        ],
    ),
    (
        "strncasecmp",
        &[
            (0, "string1", "string", false, false, true, false, false),
            (1, "string2", "string", false, false, true, false, false),
            (2, "length", "int", false, false, false, false, false),
        ],
    ),
    (
        "strncmp",
        &[
            (0, "string1", "string", false, false, true, false, false),
            (1, "string2", "string", false, false, true, false, false),
            (2, "length", "int", false, false, false, false, false),
        ],
    ),
    (
        "strpbrk",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "characters", "string", false, false, true, false, false),
        ],
    ),
    (
        "strpos",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (2, "offset", "int", false, false, false, false, false),
        ],
    ),
    (
        "strptime",
        &[
            (0, "timestamp", "string", false, false, true, false, false),
            (1, "format", "string", false, false, true, false, false),
        ],
    ),
    (
        "strrchr",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (
                2,
                "before_needle",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "strrev",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "strripos",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (2, "offset", "int", false, false, false, false, false),
        ],
    ),
    (
        "strrpos",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (2, "offset", "int", false, false, false, false, false),
        ],
    ),
    (
        "strspn",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "characters", "string", false, false, true, false, false),
            (2, "offset", "int", false, false, false, false, false),
            (3, "length", "?int", false, true, false, false, false),
        ],
    ),
    (
        "strstr",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (
                2,
                "before_needle",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "strtok",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "token", "?string", false, true, true, false, false),
        ],
    ),
    (
        "strtolower",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "strtotime",
        &[
            (0, "datetime", "string", false, false, true, false, false),
            (1, "baseTimestamp", "?int", false, true, false, false, false),
        ],
    ),
    (
        "strtoupper",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "strtr",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "from", "array|string", false, false, true, true, false),
            (2, "to", "?string", false, true, true, false, false),
        ],
    ),
    (
        "substr",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "offset", "int", false, false, false, false, false),
            (2, "length", "?int", false, true, false, false, false),
        ],
    ),
    (
        "substr_compare",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (2, "offset", "int", false, false, false, false, false),
            (3, "length", "?int", false, true, false, false, false),
            (
                4,
                "case_insensitive",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "substr_count",
        &[
            (0, "haystack", "string", false, false, true, false, false),
            (1, "needle", "string", false, false, true, false, false),
            (2, "offset", "int", false, false, false, false, false),
            (3, "length", "?int", false, true, false, false, false),
        ],
    ),
    (
        "substr_replace",
        &[
            (0, "string", "array|string", false, false, true, true, false),
            (
                1,
                "replace",
                "array|string",
                false,
                false,
                true,
                true,
                false,
            ),
            (2, "offset", "array|int", false, false, false, true, false),
            (
                3,
                "length",
                "array|int|null",
                false,
                true,
                false,
                true,
                false,
            ),
        ],
    ),
    (
        "symlink",
        &[
            (0, "target", "string", false, false, true, false, false),
            (1, "link", "string", false, false, true, false, false),
        ],
    ),
    (
        "syslog",
        &[
            (0, "priority", "int", false, false, false, false, false),
            (1, "message", "string", false, false, true, false, false),
        ],
    ),
    (
        "system",
        &[(0, "command", "string", false, false, true, false, false)],
    ),
    (
        "tan",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "tanh",
        &[(0, "num", "float", false, false, false, false, false)],
    ),
    (
        "tempnam",
        &[
            (0, "directory", "string", false, false, true, false, false),
            (1, "prefix", "string", false, false, true, false, false),
        ],
    ),
    (
        "test2",
        &[(0, "str", "string", false, false, true, false, false)],
    ),
    (
        "textdomain",
        &[(0, "domain", "?string", false, true, true, false, false)],
    ),
    (
        "tidy_get_opt_doc",
        &[(1, "option", "string", false, false, true, false, false)],
    ),
    (
        "tidy_getopt",
        &[(1, "option", "string", false, false, true, false, false)],
    ),
    (
        "tidy_parse_file",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (
                1,
                "config",
                "array|string|null",
                false,
                true,
                true,
                true,
                false,
            ),
            (2, "encoding", "?string", false, true, true, false, false),
            (
                3,
                "useIncludePath",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "tidy_parse_string",
        &[
            (0, "string", "string", false, false, true, false, false),
            (
                1,
                "config",
                "array|string|null",
                false,
                true,
                true,
                true,
                false,
            ),
            (2, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "tidy_repair_file",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (
                1,
                "config",
                "array|string|null",
                false,
                true,
                true,
                true,
                false,
            ),
            (2, "encoding", "?string", false, true, true, false, false),
            (
                3,
                "useIncludePath",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "tidy_repair_string",
        &[
            (0, "string", "string", false, false, true, false, false),
            (
                1,
                "config",
                "array|string|null",
                false,
                true,
                true,
                true,
                false,
            ),
            (2, "encoding", "?string", false, true, true, false, false),
        ],
    ),
    (
        "time_nanosleep",
        &[
            (0, "seconds", "int", false, false, false, false, false),
            (1, "nanoseconds", "int", false, false, false, false, false),
        ],
    ),
    (
        "time_sleep_until",
        &[(0, "timestamp", "float", false, false, false, false, false)],
    ),
    (
        "timezone_identifiers_list",
        &[
            (0, "timezoneGroup", "int", false, false, false, false, false),
            (1, "countryCode", "?string", false, true, true, false, false),
        ],
    ),
    (
        "timezone_name_from_abbr",
        &[
            (0, "abbr", "string", false, false, true, false, false),
            (1, "utcOffset", "int", false, false, false, false, false),
            (2, "isDST", "int", false, false, false, false, false),
        ],
    ),
    (
        "timezone_open",
        &[(0, "timezone", "string", false, false, true, false, false)],
    ),
    (
        "timezone_transitions_get",
        &[
            (
                1,
                "timestampBegin",
                "int",
                false,
                false,
                false,
                false,
                false,
            ),
            (2, "timestampEnd", "int", false, false, false, false, false),
        ],
    ),
    (
        "token_get_all",
        &[
            (0, "code", "string", false, false, true, false, false),
            (1, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "token_name",
        &[(0, "id", "int", false, false, false, false, false)],
    ),
    (
        "touch",
        &[
            (0, "filename", "string", false, false, true, false, false),
            (1, "mtime", "?int", false, true, false, false, false),
            (2, "atime", "?int", false, true, false, false, false),
        ],
    ),
    (
        "trait_exists",
        &[
            (0, "trait", "string", false, false, true, false, false),
            (1, "autoload", "bool", false, false, false, false, false),
        ],
    ),
    (
        "transliterator_create",
        &[
            (0, "id", "string", false, false, true, false, false),
            (1, "direction", "int", false, false, false, false, false),
        ],
    ),
    (
        "transliterator_create_from_rules",
        &[
            (0, "rules", "string", false, false, true, false, false),
            (1, "direction", "int", false, false, false, false, false),
        ],
    ),
    (
        "transliterator_transliterate",
        &[
            (1, "string", "string", false, false, true, false, false),
            (2, "start", "int", false, false, false, false, false),
            (3, "end", "int", false, false, false, false, false),
        ],
    ),
    (
        "trigger_error",
        &[
            (0, "message", "string", false, false, true, false, false),
            (1, "error_level", "int", false, false, false, false, false),
        ],
    ),
    (
        "trim",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "characters", "string", false, false, true, false, false),
        ],
    ),
    (
        "uasort",
        &[(0, "array", "array", false, false, false, true, true)],
    ),
    (
        "ucfirst",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "ucwords",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "separators", "string", false, false, true, false, false),
        ],
    ),
    (
        "uksort",
        &[(0, "array", "array", false, false, false, true, true)],
    ),
    (
        "umask",
        &[(0, "mask", "?int", false, true, false, false, false)],
    ),
    (
        "uniqid",
        &[
            (0, "prefix", "string", false, false, true, false, false),
            (1, "more_entropy", "bool", false, false, false, false, false),
        ],
    ),
    (
        "unixtojd",
        &[(0, "timestamp", "?int", false, true, false, false, false)],
    ),
    (
        "unlink",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "unpack",
        &[
            (0, "format", "string", false, false, true, false, false),
            (1, "string", "string", false, false, true, false, false),
            (2, "offset", "int", false, false, false, false, false),
        ],
    ),
    (
        "unserialize",
        &[
            (0, "data", "string", false, false, true, false, false),
            (1, "options", "array", false, false, false, true, false),
        ],
    ),
    (
        "urldecode",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "urlencode",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "user_error",
        &[
            (0, "message", "string", false, false, true, false, false),
            (1, "error_level", "int", false, false, false, false, false),
        ],
    ),
    (
        "usleep",
        &[(0, "microseconds", "int", false, false, false, false, false)],
    ),
    (
        "usort",
        &[(0, "array", "array", false, false, false, true, true)],
    ),
    (
        "utf8_decode",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "utf8_encode",
        &[(0, "string", "string", false, false, true, false, false)],
    ),
    (
        "var_export",
        &[(1, "return", "bool", false, false, false, false, false)],
    ),
    (
        "variant_cast",
        &[(1, "type", "int", false, false, false, false, false)],
    ),
    (
        "variant_cmp",
        &[
            (2, "locale_id", "int", false, false, false, false, false),
            (3, "flags", "int", false, false, false, false, false),
        ],
    ),
    (
        "variant_date_from_timestamp",
        &[(0, "timestamp", "int", false, false, false, false, false)],
    ),
    (
        "variant_round",
        &[(1, "decimals", "int", false, false, false, false, false)],
    ),
    (
        "variant_set_type",
        &[(1, "type", "int", false, false, false, false, false)],
    ),
    (
        "version_compare",
        &[
            (0, "version1", "string", false, false, true, false, false),
            (1, "version2", "string", false, false, true, false, false),
            (2, "operator", "?string", false, true, true, false, false),
        ],
    ),
    (
        "vfprintf",
        &[
            (1, "format", "string", false, false, true, false, false),
            (2, "values", "array", false, false, false, true, false),
        ],
    ),
    (
        "vprintf",
        &[
            (0, "format", "string", false, false, true, false, false),
            (1, "values", "array", false, false, false, true, false),
        ],
    ),
    (
        "vsprintf",
        &[
            (0, "format", "string", false, false, true, false, false),
            (1, "values", "array", false, false, false, true, false),
        ],
    ),
    (
        "wordwrap",
        &[
            (0, "string", "string", false, false, true, false, false),
            (1, "width", "int", false, false, false, false, false),
            (2, "break", "string", false, false, true, false, false),
            (
                3,
                "cut_long_words",
                "bool",
                false,
                false,
                false,
                false,
                false,
            ),
        ],
    ),
    (
        "xml_error_string",
        &[(0, "error_code", "int", false, false, false, false, false)],
    ),
    (
        "xml_parse",
        &[
            (1, "data", "string", false, false, true, false, false),
            (2, "is_final", "bool", false, false, false, false, false),
        ],
    ),
    (
        "xml_parse_into_struct",
        &[(1, "data", "string", false, false, true, false, false)],
    ),
    (
        "xml_parser_create",
        &[(0, "encoding", "?string", false, true, true, false, false)],
    ),
    (
        "xml_parser_create_ns",
        &[
            (0, "encoding", "?string", false, true, true, false, false),
            (1, "separator", "string", false, false, true, false, false),
        ],
    ),
    (
        "xml_parser_get_option",
        &[(1, "option", "int", false, false, false, false, false)],
    ),
    (
        "xml_parser_set_option",
        &[(1, "option", "int", false, false, false, false, false)],
    ),
    (
        "xmlwriter_flush",
        &[(1, "empty", "bool", false, false, false, false, false)],
    ),
    (
        "xmlwriter_open_uri",
        &[(0, "uri", "string", false, false, true, false, false)],
    ),
    (
        "xmlwriter_output_memory",
        &[(1, "flush", "bool", false, false, false, false, false)],
    ),
    (
        "xmlwriter_set_indent",
        &[(1, "enable", "bool", false, false, false, false, false)],
    ),
    (
        "xmlwriter_set_indent_string",
        &[(1, "indentation", "string", false, false, true, false, false)],
    ),
    (
        "xmlwriter_start_attribute",
        &[(1, "name", "string", false, false, true, false, false)],
    ),
    (
        "xmlwriter_start_attribute_ns",
        &[
            (1, "prefix", "?string", false, true, true, false, false),
            (2, "name", "string", false, false, true, false, false),
            (3, "namespace", "?string", false, true, true, false, false),
        ],
    ),
    (
        "xmlwriter_start_document",
        &[
            (1, "version", "?string", false, true, true, false, false),
            (2, "encoding", "?string", false, true, true, false, false),
            (3, "standalone", "?string", false, true, true, false, false),
        ],
    ),
    (
        "xmlwriter_start_dtd",
        &[
            (
                1,
                "qualifiedName",
                "string",
                false,
                false,
                true,
                false,
                false,
            ),
            (2, "publicId", "?string", false, true, true, false, false),
            (3, "systemId", "?string", false, true, true, false, false),
        ],
    ),
    (
        "xmlwriter_start_dtd_attlist",
        &[(1, "name", "string", false, false, true, false, false)],
    ),
    (
        "xmlwriter_start_dtd_element",
        &[(
            1,
            "qualifiedName",
            "string",
            false,
            false,
            true,
            false,
            false,
        )],
    ),
    (
        "xmlwriter_start_dtd_entity",
        &[
            (1, "name", "string", false, false, true, false, false),
            (2, "isParam", "bool", false, false, false, false, false),
        ],
    ),
    (
        "xmlwriter_start_element",
        &[(1, "name", "string", false, false, true, false, false)],
    ),
    (
        "xmlwriter_start_element_ns",
        &[
            (1, "prefix", "?string", false, true, true, false, false),
            (2, "name", "string", false, false, true, false, false),
            (3, "namespace", "?string", false, true, true, false, false),
        ],
    ),
    (
        "xmlwriter_start_pi",
        &[(1, "target", "string", false, false, true, false, false)],
    ),
    (
        "xmlwriter_text",
        &[(1, "content", "string", false, false, true, false, false)],
    ),
    (
        "xmlwriter_write_attribute",
        &[
            (1, "name", "string", false, false, true, false, false),
            (2, "value", "string", false, false, true, false, false),
        ],
    ),
    (
        "xmlwriter_write_attribute_ns",
        &[
            (1, "prefix", "?string", false, true, true, false, false),
            (2, "name", "string", false, false, true, false, false),
            (3, "namespace", "?string", false, true, true, false, false),
            (4, "value", "string", false, false, true, false, false),
        ],
    ),
    (
        "xmlwriter_write_cdata",
        &[(1, "content", "string", false, false, true, false, false)],
    ),
    (
        "xmlwriter_write_comment",
        &[(1, "content", "string", false, false, true, false, false)],
    ),
    (
        "xmlwriter_write_dtd",
        &[
            (1, "name", "string", false, false, true, false, false),
            (2, "publicId", "?string", false, true, true, false, false),
            (3, "systemId", "?string", false, true, true, false, false),
            (4, "content", "?string", false, true, true, false, false),
        ],
    ),
    (
        "xmlwriter_write_dtd_attlist",
        &[
            (1, "name", "string", false, false, true, false, false),
            (2, "content", "string", false, false, true, false, false),
        ],
    ),
    (
        "xmlwriter_write_dtd_element",
        &[
            (1, "name", "string", false, false, true, false, false),
            (2, "content", "string", false, false, true, false, false),
        ],
    ),
    (
        "xmlwriter_write_dtd_entity",
        &[
            (1, "name", "string", false, false, true, false, false),
            (2, "content", "string", false, false, true, false, false),
            (3, "isParam", "bool", false, false, false, false, false),
            (4, "publicId", "?string", false, true, true, false, false),
            (5, "systemId", "?string", false, true, true, false, false),
            (
                6,
                "notationData",
                "?string",
                false,
                true,
                true,
                false,
                false,
            ),
        ],
    ),
    (
        "xmlwriter_write_element",
        &[
            (1, "name", "string", false, false, true, false, false),
            (2, "content", "?string", false, true, true, false, false),
        ],
    ),
    (
        "xmlwriter_write_element_ns",
        &[
            (1, "prefix", "?string", false, true, true, false, false),
            (2, "name", "string", false, false, true, false, false),
            (3, "namespace", "?string", false, true, true, false, false),
            (4, "content", "?string", false, true, true, false, false),
        ],
    ),
    (
        "xmlwriter_write_pi",
        &[
            (1, "target", "string", false, false, true, false, false),
            (2, "content", "string", false, false, true, false, false),
        ],
    ),
    (
        "xmlwriter_write_raw",
        &[(1, "content", "string", false, false, true, false, false)],
    ),
    (
        "zip_entry_open",
        &[(2, "mode", "string", false, false, true, false, false)],
    ),
    (
        "zip_entry_read",
        &[(1, "len", "int", false, false, false, false, false)],
    ),
    (
        "zip_open",
        &[(0, "filename", "string", false, false, true, false, false)],
    ),
    (
        "zlib_decode",
        &[
            (0, "data", "string", false, false, true, false, false),
            (1, "max_length", "int", false, false, false, false, false),
        ],
    ),
    (
        "zlib_encode",
        &[
            (0, "data", "string", false, false, true, false, false),
            (1, "encoding", "int", false, false, false, false, false),
            (2, "level", "int", false, false, false, false, false),
        ],
    ),
];

/// The scalar / array typed by-value parameters of builtin `name`
/// (lowercase), from php's stubs: what zpp checks before the function runs.
pub fn params(name: &str) -> &'static [ParamInfo] {
    match PARAMS.binary_search_by(|(n, _)| (*n).cmp(name)) {
        Ok(i) => PARAMS[i].1,
        Err(_) => &[],
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sorted_and_found() {
        assert!(super::PARAMS.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(super::params("strlen")[0].1, "string");
        assert!(super::params("var_dump").is_empty());
    }
}
