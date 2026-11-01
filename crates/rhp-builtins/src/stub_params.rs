// @generated from php-src *.stub.php by the stub-param extractor; do not edit.
/// (function, [(position, name, declared type, variadic)]) for every
/// by-value parameter that accepts `string` but not objects.
pub static STRING_PARAMS: &[(&str, &[(u8, &str, &str, bool)])] = &[
    ("_", &[(0, "message", "string", false)]),
    (
        "addcslashes",
        &[
            (0, "string", "string", false),
            (1, "characters", "string", false),
        ],
    ),
    ("addslashes", &[(0, "string", "string", false)]),
    (
        "array_column",
        &[
            (1, "column_key", "string|int|null", false),
            (2, "index_key", "string|int|null", false),
        ],
    ),
    ("base64_decode", &[(0, "string", "string", false)]),
    ("base64_encode", &[(0, "string", "string", false)]),
    ("base_convert", &[(0, "num", "string", false)]),
    (
        "basename",
        &[(0, "path", "string", false), (1, "suffix", "string", false)],
    ),
    ("bin2hex", &[(0, "string", "string", false)]),
    (
        "bind_textdomain_codeset",
        &[
            (0, "domain", "string", false),
            (1, "codeset", "?string", false),
        ],
    ),
    ("bindec", &[(0, "binary_string", "string", false)]),
    (
        "bindtextdomain",
        &[
            (0, "domain", "string", false),
            (1, "directory", "?string", false),
        ],
    ),
    ("bzcompress", &[(0, "data", "string", false)]),
    ("bzdecompress", &[(0, "data", "string", false)]),
    ("bzwrite", &[(1, "data", "string", false)]),
    ("chdir", &[(0, "directory", "string", false)]),
    (
        "checkdnsrr",
        &[
            (0, "hostname", "string", false),
            (1, "type", "string", false),
        ],
    ),
    (
        "chgrp",
        &[
            (0, "filename", "string", false),
            (1, "group", "string|int", false),
        ],
    ),
    ("chmod", &[(0, "filename", "string", false)]),
    (
        "chop",
        &[
            (0, "string", "string", false),
            (1, "characters", "string", false),
        ],
    ),
    (
        "chown",
        &[
            (0, "filename", "string", false),
            (1, "user", "string|int", false),
        ],
    ),
    ("chroot", &[(0, "directory", "string", false)]),
    (
        "chunk_split",
        &[
            (0, "string", "string", false),
            (2, "separator", "string", false),
        ],
    ),
    (
        "class_alias",
        &[(0, "class", "string", false), (1, "alias", "string", false)],
    ),
    ("class_exists", &[(0, "class", "string", false)]),
    ("clearstatcache", &[(1, "filename", "string", false)]),
    (
        "collator_compare",
        &[
            (1, "string1", "string", false),
            (2, "string2", "string", false),
        ],
    ),
    ("collator_create", &[(0, "locale", "string", false)]),
    ("collator_get_sort_key", &[(1, "string", "string", false)]),
    (
        "com_event_sink",
        &[(2, "sink_interface", "array|string|null", false)],
    ),
    ("com_get_active_object", &[(0, "prog_id", "string", false)]),
    ("com_load_typelib", &[(0, "typelib", "string", false)]),
    (
        "com_print_typeinfo",
        &[(1, "dispatch_interface", "?string", false)],
    ),
    ("constant", &[(0, "name", "string", false)]),
    ("convert_uudecode", &[(0, "string", "string", false)]),
    ("convert_uuencode", &[(0, "string", "string", false)]),
    (
        "copy",
        &[(0, "from", "string", false), (1, "to", "string", false)],
    ),
    ("count_chars", &[(0, "string", "string", false)]),
    ("crc32", &[(0, "string", "string", false)]),
    (
        "crypt",
        &[(0, "string", "string", false), (1, "salt", "string", false)],
    ),
    ("curl_escape", &[(1, "string", "string", false)]),
    (
        "curl_file_create",
        &[
            (0, "filename", "string", false),
            (1, "mime_type", "?string", false),
            (2, "posted_filename", "?string", false),
        ],
    ),
    ("curl_init", &[(0, "url", "?string", false)]),
    ("curl_unescape", &[(1, "string", "string", false)]),
    ("date", &[(0, "format", "string", false)]),
    ("date_create", &[(0, "datetime", "string", false)]),
    (
        "date_create_from_format",
        &[
            (0, "format", "string", false),
            (1, "datetime", "string", false),
        ],
    ),
    ("date_create_immutable", &[(0, "datetime", "string", false)]),
    (
        "date_create_immutable_from_format",
        &[
            (0, "format", "string", false),
            (1, "datetime", "string", false),
        ],
    ),
    (
        "date_default_timezone_set",
        &[(0, "timezoneId", "string", false)],
    ),
    ("date_format", &[(1, "format", "string", false)]),
    (
        "date_interval_create_from_date_string",
        &[(0, "datetime", "string", false)],
    ),
    ("date_interval_format", &[(1, "format", "string", false)]),
    ("date_modify", &[(1, "modifier", "string", false)]),
    ("date_parse", &[(0, "datetime", "string", false)]),
    (
        "date_parse_from_format",
        &[
            (0, "format", "string", false),
            (1, "datetime", "string", false),
        ],
    ),
    (
        "datefmt_create",
        &[
            (0, "locale", "?string", false),
            (5, "pattern", "?string", false),
        ],
    ),
    ("datefmt_format_object", &[(2, "locale", "?string", false)]),
    ("datefmt_localtime", &[(1, "string", "string", false)]),
    ("datefmt_parse", &[(1, "string", "string", false)]),
    ("datefmt_set_pattern", &[(1, "pattern", "string", false)]),
    (
        "dcgettext",
        &[
            (0, "domain", "string", false),
            (1, "message", "string", false),
        ],
    ),
    (
        "dcngettext",
        &[
            (0, "domain", "string", false),
            (1, "singular", "string", false),
            (2, "plural", "string", false),
        ],
    ),
    ("define", &[(0, "constant_name", "string", false)]),
    ("defined", &[(0, "constant_name", "string", false)]),
    ("deflate_add", &[(1, "data", "string", false)]),
    (
        "dgettext",
        &[
            (0, "domain", "string", false),
            (1, "message", "string", false),
        ],
    ),
    ("die", &[(0, "status", "string|int", false)]),
    ("dir", &[(0, "directory", "string", false)]),
    ("dirname", &[(0, "path", "string", false)]),
    ("disk_free_space", &[(0, "directory", "string", false)]),
    ("disk_total_space", &[(0, "directory", "string", false)]),
    ("diskfreespace", &[(0, "directory", "string", false)]),
    ("dl", &[(0, "extension_filename", "string", false)]),
    (
        "dngettext",
        &[
            (0, "domain", "string", false),
            (1, "singular", "string", false),
            (2, "plural", "string", false),
        ],
    ),
    (
        "dns_check_record",
        &[
            (0, "hostname", "string", false),
            (1, "type", "string", false),
        ],
    ),
    ("dns_get_mx", &[(0, "hostname", "string", false)]),
    ("dns_get_record", &[(0, "hostname", "string", false)]),
    ("enchant_broker_dict_exists", &[(1, "tag", "string", false)]),
    (
        "enchant_broker_request_dict",
        &[(1, "tag", "string", false)],
    ),
    (
        "enchant_broker_request_pwl_dict",
        &[(1, "filename", "string", false)],
    ),
    (
        "enchant_broker_set_dict_path",
        &[(2, "path", "string", false)],
    ),
    (
        "enchant_broker_set_ordering",
        &[
            (1, "tag", "string", false),
            (2, "ordering", "string", false),
        ],
    ),
    ("enchant_dict_add", &[(1, "word", "string", false)]),
    (
        "enchant_dict_add_to_personal",
        &[(1, "word", "string", false)],
    ),
    (
        "enchant_dict_add_to_session",
        &[(1, "word", "string", false)],
    ),
    ("enchant_dict_check", &[(1, "word", "string", false)]),
    ("enchant_dict_is_added", &[(1, "word", "string", false)]),
    (
        "enchant_dict_is_in_session",
        &[(1, "word", "string", false)],
    ),
    ("enchant_dict_quick_check", &[(1, "word", "string", false)]),
    ("enchant_dict_remove", &[(1, "word", "string", false)]),
    (
        "enchant_dict_remove_from_session",
        &[(1, "word", "string", false)],
    ),
    (
        "enchant_dict_store_replacement",
        &[
            (1, "misspelled", "string", false),
            (2, "correct", "string", false),
        ],
    ),
    ("enchant_dict_suggest", &[(1, "word", "string", false)]),
    ("enum_exists", &[(0, "enum", "string", false)]),
    (
        "error_log",
        &[
            (0, "message", "string", false),
            (2, "destination", "?string", false),
            (3, "additional_headers", "?string", false),
        ],
    ),
    ("escapeshellarg", &[(0, "arg", "string", false)]),
    ("escapeshellcmd", &[(0, "command", "string", false)]),
    ("exec", &[(0, "command", "string", false)]),
    ("exif_imagetype", &[(0, "filename", "string", false)]),
    (
        "exif_read_data",
        &[(1, "required_sections", "?string", false)],
    ),
    ("exit", &[(0, "status", "string|int", false)]),
    (
        "explode",
        &[
            (0, "separator", "string", false),
            (1, "string", "string", false),
        ],
    ),
    ("extension_loaded", &[(0, "extension", "string", false)]),
    ("extract", &[(2, "prefix", "string", false)]),
    (
        "fgetcsv",
        &[
            (2, "separator", "string", false),
            (3, "enclosure", "string", false),
        ],
    ),
    ("file", &[(0, "filename", "string", false)]),
    ("file_exists", &[(0, "filename", "string", false)]),
    ("file_get_contents", &[(0, "filename", "string", false)]),
    ("file_put_contents", &[(0, "filename", "string", false)]),
    ("fileatime", &[(0, "filename", "string", false)]),
    ("filectime", &[(0, "filename", "string", false)]),
    ("filegroup", &[(0, "filename", "string", false)]),
    ("fileinode", &[(0, "filename", "string", false)]),
    ("filemtime", &[(0, "filename", "string", false)]),
    ("fileowner", &[(0, "filename", "string", false)]),
    ("fileperms", &[(0, "filename", "string", false)]),
    ("filesize", &[(0, "filename", "string", false)]),
    ("filetype", &[(0, "filename", "string", false)]),
    ("filter_has_var", &[(1, "var_name", "string", false)]),
    ("filter_id", &[(0, "name", "string", false)]),
    ("filter_input", &[(1, "var_name", "string", false)]),
    ("finfo_buffer", &[(1, "string", "string", false)]),
    ("finfo_file", &[(1, "filename", "string", false)]),
    ("finfo_open", &[(1, "magic_database", "?string", false)]),
    (
        "fnmatch",
        &[
            (0, "pattern", "string", false),
            (1, "filename", "string", false),
        ],
    ),
    (
        "fopen",
        &[
            (0, "filename", "string", false),
            (1, "mode", "string", false),
        ],
    ),
    ("fprintf", &[(1, "format", "string", false)]),
    (
        "fputcsv",
        &[
            (2, "separator", "string", false),
            (3, "enclosure", "string", false),
        ],
    ),
    ("fputs", &[(1, "data", "string", false)]),
    ("fscanf", &[(1, "format", "string", false)]),
    ("fsockopen", &[(0, "hostname", "string", false)]),
    (
        "ftok",
        &[
            (0, "filename", "string", false),
            (1, "project_id", "string", false),
        ],
    ),
    ("function_exists", &[(0, "function", "string", false)]),
    ("fwrite", &[(1, "data", "string", false)]),
    ("get_browser", &[(0, "user_agent", "?string", false)]),
    ("get_cfg_var", &[(0, "option", "string", false)]),
    ("get_class_vars", &[(0, "class", "string", false)]),
    ("get_extension_funcs", &[(0, "extension", "string", false)]),
    ("get_headers", &[(0, "url", "string", false)]),
    (
        "get_html_translation_table",
        &[(2, "encoding", "string", false)],
    ),
    ("get_meta_tags", &[(0, "filename", "string", false)]),
    ("get_resources", &[(0, "type", "?string", false)]),
    ("getenv", &[(0, "name", "?string", false)]),
    ("gethostbyaddr", &[(0, "ip", "string", false)]),
    ("gethostbyname", &[(0, "hostname", "string", false)]),
    ("gethostbynamel", &[(0, "hostname", "string", false)]),
    ("getimagesize", &[(0, "filename", "string", false)]),
    ("getimagesizefromstring", &[(0, "string", "string", false)]),
    ("getmxrr", &[(0, "hostname", "string", false)]),
    ("getopt", &[(0, "short_options", "string", false)]),
    ("getprotobyname", &[(0, "protocol", "string", false)]),
    (
        "getservbyname",
        &[
            (0, "service", "string", false),
            (1, "protocol", "string", false),
        ],
    ),
    ("getservbyport", &[(1, "protocol", "string", false)]),
    ("gettext", &[(0, "message", "string", false)]),
    ("glob", &[(0, "pattern", "string", false)]),
    ("gmdate", &[(0, "format", "string", false)]),
    ("gmp_import", &[(0, "data", "string", false)]),
    ("gmp_init", &[(0, "num", "string|int", false)]),
    ("gmstrftime", &[(0, "format", "string", false)]),
    ("grapheme_extract", &[(0, "haystack", "string", false)]),
    (
        "grapheme_levenshtein",
        &[
            (0, "string1", "string", false),
            (1, "string2", "string", false),
            (5, "locale", "string", false),
        ],
    ),
    ("grapheme_str_split", &[(0, "string", "string", false)]),
    (
        "grapheme_stripos",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
            (3, "locale", "string", false),
        ],
    ),
    (
        "grapheme_stristr",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
            (3, "locale", "string", false),
        ],
    ),
    ("grapheme_strlen", &[(0, "string", "string", false)]),
    (
        "grapheme_strpos",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
            (3, "locale", "string", false),
        ],
    ),
    (
        "grapheme_strripos",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
            (3, "locale", "string", false),
        ],
    ),
    (
        "grapheme_strrpos",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
            (3, "locale", "string", false),
        ],
    ),
    (
        "grapheme_strstr",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
            (3, "locale", "string", false),
        ],
    ),
    (
        "grapheme_substr",
        &[
            (0, "string", "string", false),
            (3, "locale", "string", false),
        ],
    ),
    ("gzcompress", &[(0, "data", "string", false)]),
    ("gzdecode", &[(0, "data", "string", false)]),
    ("gzdeflate", &[(0, "data", "string", false)]),
    ("gzencode", &[(0, "data", "string", false)]),
    ("gzfile", &[(0, "filename", "string", false)]),
    ("gzinflate", &[(0, "data", "string", false)]),
    (
        "gzopen",
        &[
            (0, "filename", "string", false),
            (1, "mode", "string", false),
        ],
    ),
    ("gzputs", &[(1, "data", "string", false)]),
    ("gzuncompress", &[(0, "data", "string", false)]),
    ("gzwrite", &[(1, "data", "string", false)]),
    (
        "hash",
        &[(0, "algo", "string", false), (1, "data", "string", false)],
    ),
    (
        "hash_equals",
        &[
            (0, "known_string", "string", false),
            (1, "user_string", "string", false),
        ],
    ),
    (
        "hash_file",
        &[
            (0, "algo", "string", false),
            (1, "filename", "string", false),
        ],
    ),
    (
        "hash_hkdf",
        &[
            (0, "algo", "string", false),
            (1, "key", "string", false),
            (3, "info", "string", false),
            (4, "salt", "string", false),
        ],
    ),
    (
        "hash_hmac",
        &[
            (0, "algo", "string", false),
            (1, "data", "string", false),
            (2, "key", "string", false),
        ],
    ),
    (
        "hash_hmac_file",
        &[
            (0, "algo", "string", false),
            (1, "filename", "string", false),
            (2, "key", "string", false),
        ],
    ),
    (
        "hash_init",
        &[(0, "algo", "string", false), (2, "key", "string", false)],
    ),
    (
        "hash_pbkdf2",
        &[
            (0, "algo", "string", false),
            (1, "password", "string", false),
            (2, "salt", "string", false),
        ],
    ),
    ("hash_update", &[(1, "data", "string", false)]),
    ("hash_update_file", &[(1, "filename", "string", false)]),
    ("header", &[(0, "header", "string", false)]),
    ("header_remove", &[(0, "name", "?string", false)]),
    ("hebrev", &[(0, "string", "string", false)]),
    ("hex2bin", &[(0, "string", "string", false)]),
    ("hexdec", &[(0, "hex_string", "string", false)]),
    ("highlight_file", &[(0, "filename", "string", false)]),
    ("highlight_string", &[(0, "string", "string", false)]),
    (
        "html_entity_decode",
        &[
            (0, "string", "string", false),
            (2, "encoding", "?string", false),
        ],
    ),
    (
        "htmlentities",
        &[
            (0, "string", "string", false),
            (2, "encoding", "?string", false),
        ],
    ),
    (
        "htmlspecialchars",
        &[
            (0, "string", "string", false),
            (2, "encoding", "?string", false),
        ],
    ),
    ("htmlspecialchars_decode", &[(0, "string", "string", false)]),
    (
        "http_build_query",
        &[
            (1, "numeric_prefix", "string", false),
            (2, "arg_separator", "?string", false),
        ],
    ),
    (
        "iconv",
        &[
            (0, "from_encoding", "string", false),
            (1, "to_encoding", "string", false),
            (2, "string", "string", false),
        ],
    ),
    ("iconv_get_encoding", &[(0, "type", "string", false)]),
    (
        "iconv_mime_decode",
        &[
            (0, "string", "string", false),
            (2, "encoding", "?string", false),
        ],
    ),
    (
        "iconv_mime_decode_headers",
        &[
            (0, "headers", "string", false),
            (2, "encoding", "?string", false),
        ],
    ),
    (
        "iconv_mime_encode",
        &[
            (0, "field_name", "string", false),
            (1, "field_value", "string", false),
        ],
    ),
    (
        "iconv_set_encoding",
        &[
            (0, "type", "string", false),
            (1, "encoding", "string", false),
        ],
    ),
    (
        "iconv_strlen",
        &[
            (0, "string", "string", false),
            (1, "encoding", "?string", false),
        ],
    ),
    (
        "iconv_strpos",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
            (3, "encoding", "?string", false),
        ],
    ),
    (
        "iconv_strrpos",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
            (2, "encoding", "?string", false),
        ],
    ),
    (
        "iconv_substr",
        &[
            (0, "string", "string", false),
            (3, "encoding", "?string", false),
        ],
    ),
    ("idate", &[(0, "format", "string", false)]),
    ("idn_to_ascii", &[(0, "domain", "string", false)]),
    ("idn_to_utf8", &[(0, "domain", "string", false)]),
    ("imagechar", &[(4, "char", "string", false)]),
    ("imagecharup", &[(4, "char", "string", false)]),
    ("imagecreatefromavif", &[(0, "filename", "string", false)]),
    ("imagecreatefrombmp", &[(0, "filename", "string", false)]),
    ("imagecreatefromgd", &[(0, "filename", "string", false)]),
    ("imagecreatefromgd2", &[(0, "filename", "string", false)]),
    (
        "imagecreatefromgd2part",
        &[(0, "filename", "string", false)],
    ),
    ("imagecreatefromgif", &[(0, "filename", "string", false)]),
    ("imagecreatefromjpeg", &[(0, "filename", "string", false)]),
    ("imagecreatefrompng", &[(0, "filename", "string", false)]),
    ("imagecreatefromstring", &[(0, "data", "string", false)]),
    ("imagecreatefromtga", &[(0, "filename", "string", false)]),
    ("imagecreatefromwbmp", &[(0, "filename", "string", false)]),
    ("imagecreatefromwebp", &[(0, "filename", "string", false)]),
    ("imagecreatefromxbm", &[(0, "filename", "string", false)]),
    ("imagecreatefromxpm", &[(0, "filename", "string", false)]),
    (
        "imageftbbox",
        &[
            (2, "font_filename", "string", false),
            (3, "string", "string", false),
        ],
    ),
    (
        "imagefttext",
        &[
            (6, "font_filename", "string", false),
            (7, "text", "string", false),
        ],
    ),
    ("imagegd", &[(1, "file", "?string", false)]),
    ("imagegd2", &[(1, "file", "?string", false)]),
    ("imageloadfont", &[(0, "filename", "string", false)]),
    ("imagestring", &[(4, "string", "string", false)]),
    ("imagestringup", &[(4, "string", "string", false)]),
    (
        "imagettfbbox",
        &[
            (2, "font_filename", "string", false),
            (3, "string", "string", false),
        ],
    ),
    (
        "imagettftext",
        &[
            (6, "font_filename", "string", false),
            (7, "text", "string", false),
        ],
    ),
    ("imagexbm", &[(1, "filename", "?string", false)]),
    ("implode", &[(0, "separator", "array|string", false)]),
    ("inet_ntop", &[(0, "ip", "string", false)]),
    ("inet_pton", &[(0, "ip", "string", false)]),
    ("inflate_add", &[(1, "data", "string", false)]),
    (
        "ini_alter",
        &[
            (0, "option", "string", false),
            (1, "value", "string|int|float|bool|null", false),
        ],
    ),
    ("ini_get", &[(0, "option", "string", false)]),
    ("ini_get_all", &[(0, "extension", "?string", false)]),
    ("ini_parse_quantity", &[(0, "shorthand", "string", false)]),
    ("ini_restore", &[(0, "option", "string", false)]),
    (
        "ini_set",
        &[
            (0, "option", "string", false),
            (1, "value", "string|int|float|bool|null", false),
        ],
    ),
    ("interface_exists", &[(0, "interface", "string", false)]),
    (
        "intlcal_create_instance",
        &[(1, "locale", "?string", false)],
    ),
    ("intlcal_from_date_time", &[(1, "locale", "?string", false)]),
    (
        "intlcal_get_keyword_values_for_locale",
        &[
            (0, "keyword", "string", false),
            (1, "locale", "string", false),
        ],
    ),
    (
        "intltz_count_equivalent_ids",
        &[(0, "timezoneId", "string", false)],
    ),
    (
        "intltz_create_enumeration",
        &[(0, "countryOrRawOffset", "string|int|null", false)],
    ),
    (
        "intltz_create_time_zone",
        &[(0, "timezoneId", "string", false)],
    ),
    (
        "intltz_create_time_zone_id_enumeration",
        &[(1, "region", "?string", false)],
    ),
    (
        "intltz_get_canonical_id",
        &[(0, "timezoneId", "string", false)],
    ),
    (
        "intltz_get_display_name",
        &[(3, "locale", "?string", false)],
    ),
    (
        "intltz_get_equivalent_id",
        &[(0, "timezoneId", "string", false)],
    ),
    ("intltz_get_iana_id", &[(0, "timezoneId", "string", false)]),
    (
        "intltz_get_id_for_windows_id",
        &[
            (0, "timezoneId", "string", false),
            (1, "region", "?string", false),
        ],
    ),
    ("intltz_get_region", &[(0, "timezoneId", "string", false)]),
    (
        "intltz_get_windows_id",
        &[(0, "timezoneId", "string", false)],
    ),
    ("ip2long", &[(0, "ip", "string", false)]),
    (
        "iptcembed",
        &[
            (0, "iptc_data", "string", false),
            (1, "filename", "string", false),
        ],
    ),
    ("iptcparse", &[(0, "iptc_block", "string", false)]),
    ("is_a", &[(1, "class", "string", false)]),
    ("is_dir", &[(0, "filename", "string", false)]),
    ("is_executable", &[(0, "filename", "string", false)]),
    ("is_file", &[(0, "filename", "string", false)]),
    ("is_link", &[(0, "filename", "string", false)]),
    ("is_readable", &[(0, "filename", "string", false)]),
    ("is_subclass_of", &[(1, "class", "string", false)]),
    ("is_uploaded_file", &[(0, "filename", "string", false)]),
    ("is_writable", &[(0, "filename", "string", false)]),
    ("is_writeable", &[(0, "filename", "string", false)]),
    ("join", &[(0, "separator", "array|string", false)]),
    ("json_decode", &[(0, "json", "string", false)]),
    ("json_validate", &[(0, "json", "string", false)]),
    ("lcfirst", &[(0, "string", "string", false)]),
    (
        "lchgrp",
        &[
            (0, "filename", "string", false),
            (1, "group", "string|int", false),
        ],
    ),
    (
        "lchown",
        &[
            (0, "filename", "string", false),
            (1, "user", "string|int", false),
        ],
    ),
    (
        "levenshtein",
        &[
            (0, "string1", "string", false),
            (1, "string2", "string", false),
        ],
    ),
    (
        "link",
        &[(0, "target", "string", false), (1, "link", "string", false)],
    ),
    ("linkinfo", &[(0, "path", "string", false)]),
    ("locale_accept_from_http", &[(0, "header", "string", false)]),
    (
        "locale_add_likely_subtags",
        &[(0, "locale", "string", false)],
    ),
    ("locale_canonicalize", &[(0, "locale", "string", false)]),
    (
        "locale_filter_matches",
        &[
            (0, "languageTag", "string", false),
            (1, "locale", "string", false),
        ],
    ),
    ("locale_get_all_variants", &[(0, "locale", "string", false)]),
    (
        "locale_get_display_language",
        &[
            (0, "locale", "string", false),
            (1, "displayLocale", "?string", false),
        ],
    ),
    (
        "locale_get_display_name",
        &[
            (0, "locale", "string", false),
            (1, "displayLocale", "?string", false),
        ],
    ),
    (
        "locale_get_display_region",
        &[
            (0, "locale", "string", false),
            (1, "displayLocale", "?string", false),
        ],
    ),
    (
        "locale_get_display_script",
        &[
            (0, "locale", "string", false),
            (1, "displayLocale", "?string", false),
        ],
    ),
    (
        "locale_get_display_variant",
        &[
            (0, "locale", "string", false),
            (1, "displayLocale", "?string", false),
        ],
    ),
    ("locale_get_keywords", &[(0, "locale", "string", false)]),
    (
        "locale_get_primary_language",
        &[(0, "locale", "string", false)],
    ),
    ("locale_get_region", &[(0, "locale", "string", false)]),
    ("locale_get_script", &[(0, "locale", "string", false)]),
    ("locale_is_right_to_left", &[(0, "locale", "string", false)]),
    (
        "locale_lookup",
        &[
            (1, "locale", "string", false),
            (3, "defaultLocale", "?string", false),
        ],
    ),
    ("locale_minimize_subtags", &[(0, "locale", "string", false)]),
    ("locale_parse", &[(0, "locale", "string", false)]),
    ("locale_set_default", &[(0, "locale", "string", false)]),
    ("lstat", &[(0, "filename", "string", false)]),
    (
        "ltrim",
        &[
            (0, "string", "string", false),
            (1, "characters", "string", false),
        ],
    ),
    (
        "mail",
        &[
            (0, "to", "string", false),
            (1, "subject", "string", false),
            (2, "message", "string", false),
            (3, "additional_headers", "array|string", false),
            (4, "additional_params", "string", false),
        ],
    ),
    (
        "mb_check_encoding",
        &[
            (0, "value", "array|string|null", false),
            (1, "encoding", "?string", false),
        ],
    ),
    ("mb_chr", &[(1, "encoding", "?string", false)]),
    (
        "mb_convert_case",
        &[
            (0, "string", "string", false),
            (2, "encoding", "?string", false),
        ],
    ),
    (
        "mb_convert_encoding",
        &[
            (0, "string", "array|string", false),
            (1, "to_encoding", "string", false),
            (2, "from_encoding", "array|string|null", false),
        ],
    ),
    (
        "mb_convert_kana",
        &[
            (0, "string", "string", false),
            (1, "mode", "string", false),
            (2, "encoding", "?string", false),
        ],
    ),
    (
        "mb_convert_variables",
        &[
            (0, "to_encoding", "string", false),
            (1, "from_encoding", "array|string", false),
        ],
    ),
    ("mb_decode_mimeheader", &[(0, "string", "string", false)]),
    (
        "mb_decode_numericentity",
        &[
            (0, "string", "string", false),
            (2, "encoding", "?string", false),
        ],
    ),
    (
        "mb_detect_encoding",
        &[
            (0, "string", "string", false),
            (1, "encodings", "array|string|null", false),
        ],
    ),
    (
        "mb_detect_order",
        &[(0, "encoding", "array|string|null", false)],
    ),
    (
        "mb_encode_mimeheader",
        &[
            (0, "string", "string", false),
            (1, "charset", "?string", false),
            (2, "transfer_encoding", "?string", false),
            (3, "newline", "string", false),
        ],
    ),
    (
        "mb_encode_numericentity",
        &[
            (0, "string", "string", false),
            (2, "encoding", "?string", false),
        ],
    ),
    ("mb_encoding_aliases", &[(0, "encoding", "string", false)]),
    (
        "mb_ereg",
        &[
            (0, "pattern", "string", false),
            (1, "string", "string", false),
        ],
    ),
    (
        "mb_ereg_match",
        &[
            (0, "pattern", "string", false),
            (1, "string", "string", false),
            (2, "options", "?string", false),
        ],
    ),
    (
        "mb_ereg_replace",
        &[
            (0, "pattern", "string", false),
            (1, "replacement", "string", false),
            (2, "string", "string", false),
            (3, "options", "?string", false),
        ],
    ),
    (
        "mb_ereg_replace_callback",
        &[
            (0, "pattern", "string", false),
            (2, "string", "string", false),
            (3, "options", "?string", false),
        ],
    ),
    (
        "mb_ereg_search",
        &[
            (0, "pattern", "?string", false),
            (1, "options", "?string", false),
        ],
    ),
    (
        "mb_ereg_search_init",
        &[
            (0, "string", "string", false),
            (1, "pattern", "?string", false),
            (2, "options", "?string", false),
        ],
    ),
    (
        "mb_ereg_search_pos",
        &[
            (0, "pattern", "?string", false),
            (1, "options", "?string", false),
        ],
    ),
    (
        "mb_ereg_search_regs",
        &[
            (0, "pattern", "?string", false),
            (1, "options", "?string", false),
        ],
    ),
    (
        "mb_eregi",
        &[
            (0, "pattern", "string", false),
            (1, "string", "string", false),
        ],
    ),
    (
        "mb_eregi_replace",
        &[
            (0, "pattern", "string", false),
            (1, "replacement", "string", false),
            (2, "string", "string", false),
            (3, "options", "?string", false),
        ],
    ),
    ("mb_get_info", &[(0, "type", "string", false)]),
    ("mb_http_input", &[(0, "type", "?string", false)]),
    ("mb_http_output", &[(0, "encoding", "?string", false)]),
    ("mb_internal_encoding", &[(0, "encoding", "?string", false)]),
    ("mb_language", &[(0, "language", "?string", false)]),
    (
        "mb_lcfirst",
        &[
            (0, "string", "string", false),
            (1, "encoding", "?string", false),
        ],
    ),
    (
        "mb_ltrim",
        &[
            (0, "string", "string", false),
            (1, "characters", "?string", false),
            (2, "encoding", "?string", false),
        ],
    ),
    (
        "mb_ord",
        &[
            (0, "string", "string", false),
            (1, "encoding", "?string", false),
        ],
    ),
    ("mb_output_handler", &[(0, "string", "string", false)]),
    ("mb_parse_str", &[(0, "string", "string", false)]),
    (
        "mb_preferred_mime_name",
        &[(0, "encoding", "string", false)],
    ),
    ("mb_regex_encoding", &[(0, "encoding", "?string", false)]),
    ("mb_regex_set_options", &[(0, "options", "?string", false)]),
    (
        "mb_rtrim",
        &[
            (0, "string", "string", false),
            (1, "characters", "?string", false),
            (2, "encoding", "?string", false),
        ],
    ),
    (
        "mb_scrub",
        &[
            (0, "string", "string", false),
            (1, "encoding", "?string", false),
        ],
    ),
    (
        "mb_send_mail",
        &[
            (0, "to", "string", false),
            (1, "subject", "string", false),
            (2, "message", "string", false),
            (3, "additional_headers", "array|string", false),
            (4, "additional_params", "?string", false),
        ],
    ),
    (
        "mb_split",
        &[
            (0, "pattern", "string", false),
            (1, "string", "string", false),
        ],
    ),
    (
        "mb_str_pad",
        &[
            (0, "string", "string", false),
            (2, "pad_string", "string", false),
            (4, "encoding", "?string", false),
        ],
    ),
    (
        "mb_str_split",
        &[
            (0, "string", "string", false),
            (2, "encoding", "?string", false),
        ],
    ),
    (
        "mb_strcut",
        &[
            (0, "string", "string", false),
            (3, "encoding", "?string", false),
        ],
    ),
    (
        "mb_strimwidth",
        &[
            (0, "string", "string", false),
            (3, "trim_marker", "string", false),
            (4, "encoding", "?string", false),
        ],
    ),
    (
        "mb_stripos",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
            (3, "encoding", "?string", false),
        ],
    ),
    (
        "mb_stristr",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
            (3, "encoding", "?string", false),
        ],
    ),
    (
        "mb_strlen",
        &[
            (0, "string", "string", false),
            (1, "encoding", "?string", false),
        ],
    ),
    (
        "mb_strpos",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
            (3, "encoding", "?string", false),
        ],
    ),
    (
        "mb_strrchr",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
            (3, "encoding", "?string", false),
        ],
    ),
    (
        "mb_strrichr",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
            (3, "encoding", "?string", false),
        ],
    ),
    (
        "mb_strripos",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
            (3, "encoding", "?string", false),
        ],
    ),
    (
        "mb_strrpos",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
            (3, "encoding", "?string", false),
        ],
    ),
    (
        "mb_strstr",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
            (3, "encoding", "?string", false),
        ],
    ),
    (
        "mb_strtolower",
        &[
            (0, "string", "string", false),
            (1, "encoding", "?string", false),
        ],
    ),
    (
        "mb_strtoupper",
        &[
            (0, "string", "string", false),
            (1, "encoding", "?string", false),
        ],
    ),
    (
        "mb_strwidth",
        &[
            (0, "string", "string", false),
            (1, "encoding", "?string", false),
        ],
    ),
    (
        "mb_substitute_character",
        &[(0, "substitute_character", "string|int|null", false)],
    ),
    (
        "mb_substr",
        &[
            (0, "string", "string", false),
            (3, "encoding", "?string", false),
        ],
    ),
    (
        "mb_substr_count",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
            (2, "encoding", "?string", false),
        ],
    ),
    (
        "mb_trim",
        &[
            (0, "string", "string", false),
            (1, "characters", "?string", false),
            (2, "encoding", "?string", false),
        ],
    ),
    (
        "mb_ucfirst",
        &[
            (0, "string", "string", false),
            (1, "encoding", "?string", false),
        ],
    ),
    ("md5", &[(0, "string", "string", false)]),
    ("md5_file", &[(0, "filename", "string", false)]),
    ("metaphone", &[(0, "string", "string", false)]),
    ("method_exists", &[(1, "method", "string", false)]),
    (
        "mhash",
        &[(1, "data", "string", false), (2, "key", "?string", false)],
    ),
    (
        "mhash_keygen_s2k",
        &[
            (1, "password", "string", false),
            (2, "salt", "string", false),
        ],
    ),
    ("mkdir", &[(0, "directory", "string", false)]),
    (
        "move_uploaded_file",
        &[(0, "from", "string", false), (1, "to", "string", false)],
    ),
    (
        "msgfmt_create",
        &[
            (0, "locale", "string", false),
            (1, "pattern", "string", false),
        ],
    ),
    (
        "msgfmt_format_message",
        &[
            (0, "locale", "string", false),
            (1, "pattern", "string", false),
        ],
    ),
    ("msgfmt_parse", &[(1, "string", "string", false)]),
    (
        "msgfmt_parse_message",
        &[
            (0, "locale", "string", false),
            (1, "pattern", "string", false),
            (2, "message", "string", false),
        ],
    ),
    ("msgfmt_set_pattern", &[(1, "pattern", "string", false)]),
    ("mysqli_begin_transaction", &[(2, "name", "?string", false)]),
    (
        "mysqli_change_user",
        &[
            (1, "username", "string", false),
            (2, "password", "string", false),
            (3, "database", "?string", false),
        ],
    ),
    ("mysqli_commit", &[(2, "name", "?string", false)]),
    (
        "mysqli_connect",
        &[
            (0, "hostname", "?string", false),
            (1, "username", "?string", false),
            (2, "password", "?string", false),
            (3, "database", "?string", false),
            (5, "socket", "?string", false),
        ],
    ),
    ("mysqli_debug", &[(0, "options", "string", false)]),
    ("mysqli_escape_string", &[(1, "string", "string", false)]),
    ("mysqli_execute_query", &[(1, "query", "string", false)]),
    ("mysqli_fetch_object", &[(1, "class", "string", false)]),
    ("mysqli_multi_query", &[(1, "query", "string", false)]),
    ("mysqli_prepare", &[(1, "query", "string", false)]),
    ("mysqli_query", &[(1, "query", "string", false)]),
    (
        "mysqli_real_connect",
        &[
            (1, "hostname", "?string", false),
            (2, "username", "?string", false),
            (3, "password", "?string", false),
            (4, "database", "?string", false),
            (6, "socket", "?string", false),
        ],
    ),
    (
        "mysqli_real_escape_string",
        &[(1, "string", "string", false)],
    ),
    ("mysqli_real_query", &[(1, "query", "string", false)]),
    ("mysqli_release_savepoint", &[(1, "name", "string", false)]),
    ("mysqli_rollback", &[(2, "name", "?string", false)]),
    ("mysqli_savepoint", &[(1, "name", "string", false)]),
    ("mysqli_select_db", &[(1, "database", "string", false)]),
    ("mysqli_set_charset", &[(1, "charset", "string", false)]),
    (
        "mysqli_ssl_set",
        &[
            (1, "key", "?string", false),
            (2, "certificate", "?string", false),
            (3, "ca_certificate", "?string", false),
            (4, "ca_path", "?string", false),
            (5, "cipher_algos", "?string", false),
        ],
    ),
    ("mysqli_stmt_bind_param", &[(1, "types", "string", false)]),
    ("mysqli_stmt_prepare", &[(1, "query", "string", false)]),
    (
        "mysqli_stmt_send_long_data",
        &[(2, "data", "string", false)],
    ),
    (
        "ngettext",
        &[
            (0, "singular", "string", false),
            (1, "plural", "string", false),
        ],
    ),
    ("nl2br", &[(0, "string", "string", false)]),
    (
        "normalizer_get_raw_decomposition",
        &[(0, "string", "string", false)],
    ),
    (
        "normalizer_is_normalized",
        &[(0, "string", "string", false)],
    ),
    ("normalizer_normalize", &[(0, "string", "string", false)]),
    (
        "number_format",
        &[
            (2, "decimal_separator", "?string", false),
            (3, "thousands_separator", "?string", false),
        ],
    ),
    (
        "numfmt_create",
        &[
            (0, "locale", "string", false),
            (2, "pattern", "?string", false),
        ],
    ),
    (
        "numfmt_format_currency",
        &[(2, "currency", "string", false)],
    ),
    ("numfmt_parse", &[(1, "string", "string", false)]),
    ("numfmt_parse_currency", &[(1, "string", "string", false)]),
    ("numfmt_set_pattern", &[(1, "pattern", "string", false)]),
    ("numfmt_set_symbol", &[(2, "value", "string", false)]),
    (
        "numfmt_set_text_attribute",
        &[(2, "value", "string", false)],
    ),
    ("ob_gzhandler", &[(0, "data", "string", false)]),
    ("octdec", &[(0, "octal_string", "string", false)]),
    ("opcache_compile_file", &[(0, "filename", "string", false)]),
    ("opcache_invalidate", &[(0, "filename", "string", false)]),
    (
        "opcache_is_script_cached",
        &[(0, "filename", "string", false)],
    ),
    (
        "opcache_is_script_cached_in_file_cache",
        &[(0, "filename", "string", false)],
    ),
    ("opendir", &[(0, "directory", "string", false)]),
    ("openlog", &[(0, "prefix", "string", false)]),
    (
        "openssl_cipher_iv_length",
        &[(0, "cipher_algo", "string", false)],
    ),
    (
        "openssl_cipher_key_length",
        &[(0, "cipher_algo", "string", false)],
    ),
    (
        "openssl_cms_decrypt",
        &[
            (0, "input_filename", "string", false),
            (1, "output_filename", "string", false),
        ],
    ),
    (
        "openssl_cms_encrypt",
        &[
            (0, "input_filename", "string", false),
            (1, "output_filename", "string", false),
            (6, "cipher_algo", "string|int", false),
        ],
    ),
    (
        "openssl_cms_read",
        &[(0, "input_filename", "string", false)],
    ),
    (
        "openssl_cms_sign",
        &[
            (0, "input_filename", "string", false),
            (1, "output_filename", "string", false),
            (7, "untrusted_certificates_filename", "?string", false),
        ],
    ),
    (
        "openssl_cms_verify",
        &[
            (0, "input_filename", "string", false),
            (2, "certificates", "?string", false),
            (4, "untrusted_certificates_filename", "?string", false),
            (5, "content", "?string", false),
            (6, "pk7", "?string", false),
            (7, "sigfile", "?string", false),
        ],
    ),
    (
        "openssl_csr_export_to_file",
        &[(1, "output_filename", "string", false)],
    ),
    ("openssl_csr_sign", &[(6, "serial_hex", "?string", false)]),
    (
        "openssl_decrypt",
        &[
            (0, "data", "string", false),
            (1, "cipher_algo", "string", false),
            (2, "passphrase", "string", false),
            (4, "iv", "string", false),
            (5, "tag", "?string", false),
            (6, "aad", "string", false),
        ],
    ),
    (
        "openssl_dh_compute_key",
        &[(0, "public_key", "string", false)],
    ),
    (
        "openssl_digest",
        &[
            (0, "data", "string", false),
            (1, "digest_algo", "string", false),
        ],
    ),
    (
        "openssl_encrypt",
        &[
            (0, "data", "string", false),
            (1, "cipher_algo", "string", false),
            (2, "passphrase", "string", false),
            (4, "iv", "string", false),
            (6, "aad", "string", false),
        ],
    ),
    (
        "openssl_get_privatekey",
        &[(1, "passphrase", "?string", false)],
    ),
    (
        "openssl_open",
        &[
            (0, "data", "string", false),
            (2, "encrypted_key", "string", false),
            (4, "cipher_algo", "string", false),
            (5, "iv", "?string", false),
        ],
    ),
    (
        "openssl_password_hash",
        &[
            (0, "algo", "string", false),
            (1, "password", "string", false),
        ],
    ),
    (
        "openssl_password_verify",
        &[
            (0, "algo", "string", false),
            (1, "password", "string", false),
            (2, "hash", "string", false),
        ],
    ),
    (
        "openssl_pbkdf2",
        &[
            (0, "password", "string", false),
            (1, "salt", "string", false),
            (4, "digest_algo", "string", false),
        ],
    ),
    (
        "openssl_pkcs12_export",
        &[(3, "passphrase", "string", false)],
    ),
    (
        "openssl_pkcs12_export_to_file",
        &[
            (1, "output_filename", "string", false),
            (3, "passphrase", "string", false),
        ],
    ),
    (
        "openssl_pkcs12_read",
        &[
            (0, "pkcs12", "string", false),
            (2, "passphrase", "string", false),
        ],
    ),
    (
        "openssl_pkcs7_decrypt",
        &[
            (0, "input_filename", "string", false),
            (1, "output_filename", "string", false),
        ],
    ),
    (
        "openssl_pkcs7_encrypt",
        &[
            (0, "input_filename", "string", false),
            (1, "output_filename", "string", false),
        ],
    ),
    ("openssl_pkcs7_read", &[(0, "data", "string", false)]),
    (
        "openssl_pkcs7_sign",
        &[
            (0, "input_filename", "string", false),
            (1, "output_filename", "string", false),
            (6, "untrusted_certificates_filename", "?string", false),
        ],
    ),
    (
        "openssl_pkcs7_verify",
        &[
            (0, "input_filename", "string", false),
            (2, "signers_certificates_filename", "?string", false),
            (4, "untrusted_certificates_filename", "?string", false),
            (5, "content", "?string", false),
            (6, "output_filename", "?string", false),
        ],
    ),
    (
        "openssl_pkey_export",
        &[(2, "passphrase", "?string", false)],
    ),
    (
        "openssl_pkey_export_to_file",
        &[
            (1, "output_filename", "string", false),
            (2, "passphrase", "?string", false),
        ],
    ),
    (
        "openssl_pkey_get_private",
        &[(1, "passphrase", "?string", false)],
    ),
    (
        "openssl_private_decrypt",
        &[
            (0, "data", "string", false),
            (4, "digest_algo", "?string", false),
        ],
    ),
    ("openssl_private_encrypt", &[(0, "data", "string", false)]),
    ("openssl_public_decrypt", &[(0, "data", "string", false)]),
    (
        "openssl_public_encrypt",
        &[
            (0, "data", "string", false),
            (4, "digest_algo", "?string", false),
        ],
    ),
    (
        "openssl_seal",
        &[
            (0, "data", "string", false),
            (4, "cipher_algo", "string", false),
        ],
    ),
    (
        "openssl_sign",
        &[
            (0, "data", "string", false),
            (3, "algorithm", "string|int", false),
        ],
    ),
    ("openssl_spki_export", &[(0, "spki", "string", false)]),
    (
        "openssl_spki_export_challenge",
        &[(0, "spki", "string", false)],
    ),
    ("openssl_spki_new", &[(1, "challenge", "string", false)]),
    ("openssl_spki_verify", &[(0, "spki", "string", false)]),
    (
        "openssl_verify",
        &[
            (0, "data", "string", false),
            (1, "signature", "string", false),
            (3, "algorithm", "string|int", false),
        ],
    ),
    (
        "openssl_x509_checkpurpose",
        &[(3, "untrusted_certificates_file", "?string", false)],
    ),
    (
        "openssl_x509_export_to_file",
        &[(1, "output_filename", "string", false)],
    ),
    (
        "openssl_x509_fingerprint",
        &[(1, "digest_algo", "string", false)],
    ),
    ("ord", &[(0, "character", "string", false)]),
    (
        "output_add_rewrite_var",
        &[(0, "name", "string", false), (1, "value", "string", false)],
    ),
    ("pack", &[(0, "format", "string", false)]),
    ("parse_ini_file", &[(0, "filename", "string", false)]),
    ("parse_ini_string", &[(0, "ini_string", "string", false)]),
    ("parse_str", &[(0, "string", "string", false)]),
    ("parse_url", &[(0, "url", "string", false)]),
    ("passthru", &[(0, "command", "string", false)]),
    ("password_get_info", &[(0, "hash", "string", false)]),
    (
        "password_hash",
        &[
            (0, "password", "string", false),
            (1, "algo", "string|int|null", false),
        ],
    ),
    (
        "password_needs_rehash",
        &[
            (0, "hash", "string", false),
            (1, "algo", "string|int|null", false),
        ],
    ),
    (
        "password_verify",
        &[
            (0, "password", "string", false),
            (1, "hash", "string", false),
        ],
    ),
    ("pathinfo", &[(0, "path", "string", false)]),
    ("pfsockopen", &[(0, "hostname", "string", false)]),
    ("php_strip_whitespace", &[(0, "filename", "string", false)]),
    ("php_uname", &[(0, "mode", "string", false)]),
    ("phpversion", &[(0, "extension", "?string", false)]),
    (
        "popen",
        &[
            (0, "command", "string", false),
            (1, "mode", "string", false),
        ],
    ),
    ("posix_access", &[(0, "filename", "string", false)]),
    ("posix_eaccess", &[(0, "filename", "string", false)]),
    ("posix_getgrnam", &[(0, "name", "string", false)]),
    ("posix_getpwnam", &[(0, "username", "string", false)]),
    ("posix_initgroups", &[(0, "username", "string", false)]),
    ("posix_mkfifo", &[(0, "filename", "string", false)]),
    ("posix_mknod", &[(0, "filename", "string", false)]),
    ("posix_pathconf", &[(0, "path", "string", false)]),
    (
        "preg_filter",
        &[
            (0, "pattern", "array|string", false),
            (1, "replacement", "array|string", false),
            (2, "subject", "array|string", false),
        ],
    ),
    ("preg_grep", &[(0, "pattern", "string", false)]),
    (
        "preg_match",
        &[
            (0, "pattern", "string", false),
            (1, "subject", "string", false),
        ],
    ),
    (
        "preg_match_all",
        &[
            (0, "pattern", "string", false),
            (1, "subject", "string", false),
        ],
    ),
    (
        "preg_quote",
        &[
            (0, "str", "string", false),
            (1, "delimiter", "?string", false),
        ],
    ),
    (
        "preg_replace",
        &[
            (0, "pattern", "array|string", false),
            (1, "replacement", "array|string", false),
            (2, "subject", "array|string", false),
        ],
    ),
    (
        "preg_replace_callback",
        &[
            (0, "pattern", "array|string", false),
            (2, "subject", "array|string", false),
        ],
    ),
    (
        "preg_replace_callback_array",
        &[(1, "subject", "array|string", false)],
    ),
    (
        "preg_split",
        &[
            (0, "pattern", "string", false),
            (1, "subject", "string", false),
        ],
    ),
    ("printf", &[(0, "format", "string", false)]),
    (
        "proc_open",
        &[
            (0, "command", "array|string", false),
            (3, "cwd", "?string", false),
        ],
    ),
    ("property_exists", &[(1, "property", "string", false)]),
    ("putenv", &[(0, "assignment", "string", false)]),
    ("quoted_printable_decode", &[(0, "string", "string", false)]),
    ("quoted_printable_encode", &[(0, "string", "string", false)]),
    ("quotemeta", &[(0, "string", "string", false)]),
    (
        "range",
        &[
            (0, "start", "string|int|float", false),
            (1, "end", "string|int|float", false),
        ],
    ),
    ("rawurldecode", &[(0, "string", "string", false)]),
    ("rawurlencode", &[(0, "string", "string", false)]),
    ("readfile", &[(0, "filename", "string", false)]),
    ("readgzfile", &[(0, "filename", "string", false)]),
    ("readline", &[(0, "prompt", "?string", false)]),
    ("readline_add_history", &[(0, "prompt", "string", false)]),
    (
        "readline_callback_handler_install",
        &[(0, "prompt", "string", false)],
    ),
    ("readline_info", &[(0, "var_name", "?string", false)]),
    (
        "readline_read_history",
        &[(0, "filename", "?string", false)],
    ),
    (
        "readline_write_history",
        &[(0, "filename", "?string", false)],
    ),
    ("readlink", &[(0, "path", "string", false)]),
    ("realpath", &[(0, "path", "string", false)]),
    (
        "rename",
        &[(0, "from", "string", false), (1, "to", "string", false)],
    ),
    (
        "resourcebundle_create",
        &[
            (0, "locale", "?string", false),
            (1, "bundle", "?string", false),
        ],
    ),
    ("resourcebundle_get", &[(1, "index", "string|int", false)]),
    ("resourcebundle_locales", &[(0, "bundle", "string", false)]),
    ("rmdir", &[(0, "directory", "string", false)]),
    (
        "rtrim",
        &[
            (0, "string", "string", false),
            (1, "characters", "string", false),
        ],
    ),
    (
        "sapi_windows_cp_conv",
        &[
            (0, "in_codepage", "string|int", false),
            (1, "out_codepage", "string|int", false),
            (2, "subject", "string", false),
        ],
    ),
    ("sapi_windows_cp_get", &[(0, "kind", "string", false)]),
    ("scandir", &[(0, "directory", "string", false)]),
    ("session_cache_limiter", &[(0, "value", "?string", false)]),
    ("session_create_id", &[(0, "prefix", "string", false)]),
    ("session_decode", &[(0, "data", "string", false)]),
    ("session_id", &[(0, "id", "?string", false)]),
    ("session_module_name", &[(0, "module", "?string", false)]),
    ("session_name", &[(0, "name", "?string", false)]),
    ("session_save_path", &[(0, "path", "?string", false)]),
    (
        "session_set_cookie_params",
        &[
            (1, "path", "?string", false),
            (2, "domain", "?string", false),
        ],
    ),
    ("set_include_path", &[(0, "include_path", "string", false)]),
    (
        "setcookie",
        &[
            (0, "name", "string", false),
            (1, "value", "string", false),
            (3, "path", "string", false),
            (4, "domain", "string", false),
        ],
    ),
    (
        "setrawcookie",
        &[
            (0, "name", "string", false),
            (1, "value", "string", false),
            (3, "path", "string", false),
            (4, "domain", "string", false),
        ],
    ),
    ("settype", &[(1, "type", "string", false)]),
    ("sha1", &[(0, "string", "string", false)]),
    ("sha1_file", &[(0, "filename", "string", false)]),
    ("shell_exec", &[(0, "command", "string", false)]),
    ("shmop_open", &[(1, "mode", "string", false)]),
    ("shmop_write", &[(1, "data", "string", false)]),
    ("show_source", &[(0, "filename", "string", false)]),
    (
        "similar_text",
        &[
            (0, "string1", "string", false),
            (1, "string2", "string", false),
        ],
    ),
    (
        "simplexml_import_dom",
        &[(1, "class_name", "?string", false)],
    ),
    (
        "simplexml_load_file",
        &[
            (0, "filename", "string", false),
            (1, "class_name", "?string", false),
            (3, "namespace_or_prefix", "string", false),
        ],
    ),
    (
        "simplexml_load_string",
        &[
            (0, "data", "string", false),
            (1, "class_name", "?string", false),
            (3, "namespace_or_prefix", "string", false),
        ],
    ),
    (
        "snmp2_get",
        &[
            (0, "hostname", "string", false),
            (1, "community", "string", false),
            (2, "object_id", "array|string", false),
        ],
    ),
    (
        "snmp2_getnext",
        &[
            (0, "hostname", "string", false),
            (1, "community", "string", false),
            (2, "object_id", "array|string", false),
        ],
    ),
    (
        "snmp2_real_walk",
        &[
            (0, "hostname", "string", false),
            (1, "community", "string", false),
            (2, "object_id", "array|string", false),
        ],
    ),
    (
        "snmp2_set",
        &[
            (0, "hostname", "string", false),
            (1, "community", "string", false),
            (2, "object_id", "array|string", false),
            (3, "type", "array|string", false),
            (4, "value", "array|string", false),
        ],
    ),
    (
        "snmp2_walk",
        &[
            (0, "hostname", "string", false),
            (1, "community", "string", false),
            (2, "object_id", "array|string", false),
        ],
    ),
    (
        "snmp3_get",
        &[
            (0, "hostname", "string", false),
            (1, "security_name", "string", false),
            (2, "security_level", "string", false),
            (3, "auth_protocol", "string", false),
            (4, "auth_passphrase", "string", false),
            (5, "privacy_protocol", "string", false),
            (6, "privacy_passphrase", "string", false),
            (7, "object_id", "array|string", false),
        ],
    ),
    (
        "snmp3_getnext",
        &[
            (0, "hostname", "string", false),
            (1, "security_name", "string", false),
            (2, "security_level", "string", false),
            (3, "auth_protocol", "string", false),
            (4, "auth_passphrase", "string", false),
            (5, "privacy_protocol", "string", false),
            (6, "privacy_passphrase", "string", false),
            (7, "object_id", "array|string", false),
        ],
    ),
    (
        "snmp3_real_walk",
        &[
            (0, "hostname", "string", false),
            (1, "security_name", "string", false),
            (2, "security_level", "string", false),
            (3, "auth_protocol", "string", false),
            (4, "auth_passphrase", "string", false),
            (5, "privacy_protocol", "string", false),
            (6, "privacy_passphrase", "string", false),
            (7, "object_id", "array|string", false),
        ],
    ),
    (
        "snmp3_set",
        &[
            (0, "hostname", "string", false),
            (1, "security_name", "string", false),
            (2, "security_level", "string", false),
            (3, "auth_protocol", "string", false),
            (4, "auth_passphrase", "string", false),
            (5, "privacy_protocol", "string", false),
            (6, "privacy_passphrase", "string", false),
            (7, "object_id", "array|string", false),
            (8, "type", "array|string", false),
            (9, "value", "array|string", false),
        ],
    ),
    (
        "snmp3_walk",
        &[
            (0, "hostname", "string", false),
            (1, "security_name", "string", false),
            (2, "security_level", "string", false),
            (3, "auth_protocol", "string", false),
            (4, "auth_passphrase", "string", false),
            (5, "privacy_protocol", "string", false),
            (6, "privacy_passphrase", "string", false),
            (7, "object_id", "array|string", false),
        ],
    ),
    ("snmp_read_mib", &[(0, "filename", "string", false)]),
    (
        "snmpget",
        &[
            (0, "hostname", "string", false),
            (1, "community", "string", false),
            (2, "object_id", "array|string", false),
        ],
    ),
    (
        "snmpgetnext",
        &[
            (0, "hostname", "string", false),
            (1, "community", "string", false),
            (2, "object_id", "array|string", false),
        ],
    ),
    (
        "snmprealwalk",
        &[
            (0, "hostname", "string", false),
            (1, "community", "string", false),
            (2, "object_id", "array|string", false),
        ],
    ),
    (
        "snmpset",
        &[
            (0, "hostname", "string", false),
            (1, "community", "string", false),
            (2, "object_id", "array|string", false),
            (3, "type", "array|string", false),
            (4, "value", "array|string", false),
        ],
    ),
    (
        "snmpwalk",
        &[
            (0, "hostname", "string", false),
            (1, "community", "string", false),
            (2, "object_id", "array|string", false),
        ],
    ),
    (
        "snmpwalkoid",
        &[
            (0, "hostname", "string", false),
            (1, "community", "string", false),
            (2, "object_id", "array|string", false),
        ],
    ),
    (
        "socket_addrinfo_lookup",
        &[
            (0, "host", "string", false),
            (1, "service", "?string", false),
        ],
    ),
    ("socket_bind", &[(1, "address", "string", false)]),
    ("socket_connect", &[(1, "address", "string", false)]),
    ("socket_send", &[(1, "data", "string", false)]),
    (
        "socket_sendto",
        &[
            (1, "data", "string", false),
            (4, "address", "string", false),
        ],
    ),
    ("socket_write", &[(1, "data", "string", false)]),
    (
        "socket_wsaprotocol_info_import",
        &[(0, "info_id", "string", false)],
    ),
    (
        "socket_wsaprotocol_info_release",
        &[(0, "info_id", "string", false)],
    ),
    ("sodium_add", &[(1, "string2", "string", false)]),
    (
        "sodium_base642bin",
        &[
            (0, "string", "string", false),
            (2, "ignore", "string", false),
        ],
    ),
    ("sodium_bin2base64", &[(0, "string", "string", false)]),
    ("sodium_bin2hex", &[(0, "string", "string", false)]),
    (
        "sodium_compare",
        &[
            (0, "string1", "string", false),
            (1, "string2", "string", false),
        ],
    ),
    (
        "sodium_crypto_aead_aegis128l_decrypt",
        &[
            (0, "ciphertext", "string", false),
            (1, "additional_data", "string", false),
            (2, "nonce", "string", false),
            (3, "key", "string", false),
        ],
    ),
    (
        "sodium_crypto_aead_aegis128l_encrypt",
        &[
            (0, "message", "string", false),
            (1, "additional_data", "string", false),
            (2, "nonce", "string", false),
            (3, "key", "string", false),
        ],
    ),
    (
        "sodium_crypto_aead_aegis256_decrypt",
        &[
            (0, "ciphertext", "string", false),
            (1, "additional_data", "string", false),
            (2, "nonce", "string", false),
            (3, "key", "string", false),
        ],
    ),
    (
        "sodium_crypto_aead_aegis256_encrypt",
        &[
            (0, "message", "string", false),
            (1, "additional_data", "string", false),
            (2, "nonce", "string", false),
            (3, "key", "string", false),
        ],
    ),
    (
        "sodium_crypto_aead_aes256gcm_decrypt",
        &[
            (0, "ciphertext", "string", false),
            (1, "additional_data", "string", false),
            (2, "nonce", "string", false),
            (3, "key", "string", false),
        ],
    ),
    (
        "sodium_crypto_aead_aes256gcm_encrypt",
        &[
            (0, "message", "string", false),
            (1, "additional_data", "string", false),
            (2, "nonce", "string", false),
            (3, "key", "string", false),
        ],
    ),
    (
        "sodium_crypto_aead_chacha20poly1305_decrypt",
        &[
            (0, "ciphertext", "string", false),
            (1, "additional_data", "string", false),
            (2, "nonce", "string", false),
            (3, "key", "string", false),
        ],
    ),
    (
        "sodium_crypto_aead_chacha20poly1305_encrypt",
        &[
            (0, "message", "string", false),
            (1, "additional_data", "string", false),
            (2, "nonce", "string", false),
            (3, "key", "string", false),
        ],
    ),
    (
        "sodium_crypto_aead_chacha20poly1305_ietf_decrypt",
        &[
            (0, "ciphertext", "string", false),
            (1, "additional_data", "string", false),
            (2, "nonce", "string", false),
            (3, "key", "string", false),
        ],
    ),
    (
        "sodium_crypto_aead_chacha20poly1305_ietf_encrypt",
        &[
            (0, "message", "string", false),
            (1, "additional_data", "string", false),
            (2, "nonce", "string", false),
            (3, "key", "string", false),
        ],
    ),
    (
        "sodium_crypto_aead_xchacha20poly1305_ietf_decrypt",
        &[
            (0, "ciphertext", "string", false),
            (1, "additional_data", "string", false),
            (2, "nonce", "string", false),
            (3, "key", "string", false),
        ],
    ),
    (
        "sodium_crypto_aead_xchacha20poly1305_ietf_encrypt",
        &[
            (0, "message", "string", false),
            (1, "additional_data", "string", false),
            (2, "nonce", "string", false),
            (3, "key", "string", false),
        ],
    ),
    (
        "sodium_crypto_auth",
        &[(0, "message", "string", false), (1, "key", "string", false)],
    ),
    (
        "sodium_crypto_auth_verify",
        &[
            (0, "mac", "string", false),
            (1, "message", "string", false),
            (2, "key", "string", false),
        ],
    ),
    (
        "sodium_crypto_box",
        &[
            (0, "message", "string", false),
            (1, "nonce", "string", false),
            (2, "key_pair", "string", false),
        ],
    ),
    (
        "sodium_crypto_box_keypair_from_secretkey_and_publickey",
        &[
            (0, "secret_key", "string", false),
            (1, "public_key", "string", false),
        ],
    ),
    (
        "sodium_crypto_box_open",
        &[
            (0, "ciphertext", "string", false),
            (1, "nonce", "string", false),
            (2, "key_pair", "string", false),
        ],
    ),
    (
        "sodium_crypto_box_publickey",
        &[(0, "key_pair", "string", false)],
    ),
    (
        "sodium_crypto_box_publickey_from_secretkey",
        &[(0, "secret_key", "string", false)],
    ),
    (
        "sodium_crypto_box_seal",
        &[
            (0, "message", "string", false),
            (1, "public_key", "string", false),
        ],
    ),
    (
        "sodium_crypto_box_seal_open",
        &[
            (0, "ciphertext", "string", false),
            (1, "key_pair", "string", false),
        ],
    ),
    (
        "sodium_crypto_box_secretkey",
        &[(0, "key_pair", "string", false)],
    ),
    (
        "sodium_crypto_box_seed_keypair",
        &[(0, "seed", "string", false)],
    ),
    (
        "sodium_crypto_core_ristretto255_add",
        &[(0, "p", "string", false), (1, "q", "string", false)],
    ),
    (
        "sodium_crypto_core_ristretto255_from_hash",
        &[(0, "s", "string", false)],
    ),
    (
        "sodium_crypto_core_ristretto255_is_valid_point",
        &[(0, "s", "string", false)],
    ),
    (
        "sodium_crypto_core_ristretto255_scalar_add",
        &[(0, "x", "string", false), (1, "y", "string", false)],
    ),
    (
        "sodium_crypto_core_ristretto255_scalar_complement",
        &[(0, "s", "string", false)],
    ),
    (
        "sodium_crypto_core_ristretto255_scalar_invert",
        &[(0, "s", "string", false)],
    ),
    (
        "sodium_crypto_core_ristretto255_scalar_mul",
        &[(0, "x", "string", false), (1, "y", "string", false)],
    ),
    (
        "sodium_crypto_core_ristretto255_scalar_negate",
        &[(0, "s", "string", false)],
    ),
    (
        "sodium_crypto_core_ristretto255_scalar_reduce",
        &[(0, "s", "string", false)],
    ),
    (
        "sodium_crypto_core_ristretto255_scalar_sub",
        &[(0, "x", "string", false), (1, "y", "string", false)],
    ),
    (
        "sodium_crypto_core_ristretto255_sub",
        &[(0, "p", "string", false), (1, "q", "string", false)],
    ),
    (
        "sodium_crypto_generichash",
        &[(0, "message", "string", false), (1, "key", "string", false)],
    ),
    (
        "sodium_crypto_generichash_init",
        &[(0, "key", "string", false)],
    ),
    (
        "sodium_crypto_generichash_update",
        &[(1, "message", "string", false)],
    ),
    (
        "sodium_crypto_kdf_derive_from_key",
        &[(2, "context", "string", false), (3, "key", "string", false)],
    ),
    (
        "sodium_crypto_kx_client_session_keys",
        &[
            (0, "client_key_pair", "string", false),
            (1, "server_key", "string", false),
        ],
    ),
    (
        "sodium_crypto_kx_publickey",
        &[(0, "key_pair", "string", false)],
    ),
    (
        "sodium_crypto_kx_secretkey",
        &[(0, "key_pair", "string", false)],
    ),
    (
        "sodium_crypto_kx_seed_keypair",
        &[(0, "seed", "string", false)],
    ),
    (
        "sodium_crypto_kx_server_session_keys",
        &[
            (0, "server_key_pair", "string", false),
            (1, "client_key", "string", false),
        ],
    ),
    (
        "sodium_crypto_pwhash",
        &[
            (1, "password", "string", false),
            (2, "salt", "string", false),
        ],
    ),
    (
        "sodium_crypto_pwhash_scryptsalsa208sha256",
        &[
            (1, "password", "string", false),
            (2, "salt", "string", false),
        ],
    ),
    (
        "sodium_crypto_pwhash_scryptsalsa208sha256_str",
        &[(0, "password", "string", false)],
    ),
    (
        "sodium_crypto_pwhash_scryptsalsa208sha256_str_verify",
        &[
            (0, "hash", "string", false),
            (1, "password", "string", false),
        ],
    ),
    (
        "sodium_crypto_pwhash_str",
        &[(0, "password", "string", false)],
    ),
    (
        "sodium_crypto_pwhash_str_needs_rehash",
        &[(0, "password", "string", false)],
    ),
    (
        "sodium_crypto_pwhash_str_verify",
        &[
            (0, "hash", "string", false),
            (1, "password", "string", false),
        ],
    ),
    (
        "sodium_crypto_scalarmult",
        &[(0, "n", "string", false), (1, "p", "string", false)],
    ),
    (
        "sodium_crypto_scalarmult_base",
        &[(0, "secret_key", "string", false)],
    ),
    (
        "sodium_crypto_scalarmult_ristretto255",
        &[(0, "n", "string", false), (1, "p", "string", false)],
    ),
    (
        "sodium_crypto_scalarmult_ristretto255_base",
        &[(0, "n", "string", false)],
    ),
    (
        "sodium_crypto_secretbox",
        &[
            (0, "message", "string", false),
            (1, "nonce", "string", false),
            (2, "key", "string", false),
        ],
    ),
    (
        "sodium_crypto_secretbox_open",
        &[
            (0, "ciphertext", "string", false),
            (1, "nonce", "string", false),
            (2, "key", "string", false),
        ],
    ),
    (
        "sodium_crypto_secretstream_xchacha20poly1305_init_pull",
        &[(0, "header", "string", false), (1, "key", "string", false)],
    ),
    (
        "sodium_crypto_secretstream_xchacha20poly1305_init_push",
        &[(0, "key", "string", false)],
    ),
    (
        "sodium_crypto_secretstream_xchacha20poly1305_pull",
        &[
            (1, "ciphertext", "string", false),
            (2, "additional_data", "string", false),
        ],
    ),
    (
        "sodium_crypto_secretstream_xchacha20poly1305_push",
        &[
            (1, "message", "string", false),
            (2, "additional_data", "string", false),
        ],
    ),
    (
        "sodium_crypto_shorthash",
        &[(0, "message", "string", false), (1, "key", "string", false)],
    ),
    (
        "sodium_crypto_sign",
        &[
            (0, "message", "string", false),
            (1, "secret_key", "string", false),
        ],
    ),
    (
        "sodium_crypto_sign_detached",
        &[
            (0, "message", "string", false),
            (1, "secret_key", "string", false),
        ],
    ),
    (
        "sodium_crypto_sign_ed25519_pk_to_curve25519",
        &[(0, "public_key", "string", false)],
    ),
    (
        "sodium_crypto_sign_ed25519_sk_to_curve25519",
        &[(0, "secret_key", "string", false)],
    ),
    (
        "sodium_crypto_sign_keypair_from_secretkey_and_publickey",
        &[
            (0, "secret_key", "string", false),
            (1, "public_key", "string", false),
        ],
    ),
    (
        "sodium_crypto_sign_open",
        &[
            (0, "signed_message", "string", false),
            (1, "public_key", "string", false),
        ],
    ),
    (
        "sodium_crypto_sign_publickey",
        &[(0, "key_pair", "string", false)],
    ),
    (
        "sodium_crypto_sign_publickey_from_secretkey",
        &[(0, "secret_key", "string", false)],
    ),
    (
        "sodium_crypto_sign_secretkey",
        &[(0, "key_pair", "string", false)],
    ),
    (
        "sodium_crypto_sign_seed_keypair",
        &[(0, "seed", "string", false)],
    ),
    (
        "sodium_crypto_sign_verify_detached",
        &[
            (0, "signature", "string", false),
            (1, "message", "string", false),
            (2, "public_key", "string", false),
        ],
    ),
    (
        "sodium_crypto_stream",
        &[(1, "nonce", "string", false), (2, "key", "string", false)],
    ),
    (
        "sodium_crypto_stream_xchacha20",
        &[(1, "nonce", "string", false), (2, "key", "string", false)],
    ),
    (
        "sodium_crypto_stream_xchacha20_xor",
        &[
            (0, "message", "string", false),
            (1, "nonce", "string", false),
            (2, "key", "string", false),
        ],
    ),
    (
        "sodium_crypto_stream_xchacha20_xor_ic",
        &[
            (0, "message", "string", false),
            (1, "nonce", "string", false),
            (3, "key", "string", false),
        ],
    ),
    (
        "sodium_crypto_stream_xor",
        &[
            (0, "message", "string", false),
            (1, "nonce", "string", false),
            (2, "key", "string", false),
        ],
    ),
    (
        "sodium_hex2bin",
        &[
            (0, "string", "string", false),
            (1, "ignore", "string", false),
        ],
    ),
    (
        "sodium_memcmp",
        &[
            (0, "string1", "string", false),
            (1, "string2", "string", false),
        ],
    ),
    ("sodium_pad", &[(0, "string", "string", false)]),
    ("sodium_unpad", &[(0, "string", "string", false)]),
    ("soundex", &[(0, "string", "string", false)]),
    (
        "spl_autoload",
        &[
            (0, "class", "string", false),
            (1, "file_extensions", "?string", false),
        ],
    ),
    ("spl_autoload_call", &[(0, "class", "string", false)]),
    (
        "spl_autoload_extensions",
        &[(0, "file_extensions", "?string", false)],
    ),
    ("sprintf", &[(0, "format", "string", false)]),
    (
        "sscanf",
        &[
            (0, "string", "string", false),
            (1, "format", "string", false),
        ],
    ),
    ("stat", &[(0, "filename", "string", false)]),
    (
        "str_contains",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
        ],
    ),
    ("str_decrement", &[(0, "string", "string", false)]),
    (
        "str_ends_with",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
        ],
    ),
    (
        "str_getcsv",
        &[
            (0, "string", "string", false),
            (1, "separator", "string", false),
            (2, "enclosure", "string", false),
        ],
    ),
    ("str_increment", &[(0, "string", "string", false)]),
    (
        "str_ireplace",
        &[
            (0, "search", "array|string", false),
            (1, "replace", "array|string", false),
            (2, "subject", "array|string", false),
        ],
    ),
    (
        "str_pad",
        &[
            (0, "string", "string", false),
            (2, "pad_string", "string", false),
        ],
    ),
    ("str_repeat", &[(0, "string", "string", false)]),
    (
        "str_replace",
        &[
            (0, "search", "array|string", false),
            (1, "replace", "array|string", false),
            (2, "subject", "array|string", false),
        ],
    ),
    ("str_rot13", &[(0, "string", "string", false)]),
    ("str_shuffle", &[(0, "string", "string", false)]),
    ("str_split", &[(0, "string", "string", false)]),
    (
        "str_starts_with",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
        ],
    ),
    (
        "str_word_count",
        &[
            (0, "string", "string", false),
            (2, "characters", "?string", false),
        ],
    ),
    (
        "strcasecmp",
        &[
            (0, "string1", "string", false),
            (1, "string2", "string", false),
        ],
    ),
    (
        "strchr",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
        ],
    ),
    (
        "strcmp",
        &[
            (0, "string1", "string", false),
            (1, "string2", "string", false),
        ],
    ),
    (
        "strcoll",
        &[
            (0, "string1", "string", false),
            (1, "string2", "string", false),
        ],
    ),
    (
        "strcspn",
        &[
            (0, "string", "string", false),
            (1, "characters", "string", false),
        ],
    ),
    ("stream_bucket_new", &[(1, "buffer", "string", false)]),
    (
        "stream_context_set_option",
        &[
            (1, "wrapper_or_options", "array|string", false),
            (2, "option_name", "?string", false),
        ],
    ),
    (
        "stream_filter_append",
        &[(1, "filter_name", "string", false)],
    ),
    (
        "stream_filter_prepend",
        &[(1, "filter_name", "string", false)],
    ),
    (
        "stream_filter_register",
        &[
            (0, "filter_name", "string", false),
            (1, "class", "string", false),
        ],
    ),
    ("stream_get_line", &[(2, "ending", "string", false)]),
    (
        "stream_register_wrapper",
        &[
            (0, "protocol", "string", false),
            (1, "class", "string", false),
        ],
    ),
    (
        "stream_resolve_include_path",
        &[(0, "filename", "string", false)],
    ),
    ("stream_socket_client", &[(0, "address", "string", false)]),
    (
        "stream_socket_sendto",
        &[
            (1, "data", "string", false),
            (3, "address", "string", false),
        ],
    ),
    ("stream_socket_server", &[(0, "address", "string", false)]),
    (
        "stream_wrapper_register",
        &[
            (0, "protocol", "string", false),
            (1, "class", "string", false),
        ],
    ),
    (
        "stream_wrapper_restore",
        &[(0, "protocol", "string", false)],
    ),
    (
        "stream_wrapper_unregister",
        &[(0, "protocol", "string", false)],
    ),
    ("strftime", &[(0, "format", "string", false)]),
    (
        "strip_tags",
        &[
            (0, "string", "string", false),
            (1, "allowed_tags", "array|string|null", false),
        ],
    ),
    ("stripcslashes", &[(0, "string", "string", false)]),
    (
        "stripos",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
        ],
    ),
    ("stripslashes", &[(0, "string", "string", false)]),
    (
        "stristr",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
        ],
    ),
    ("strlen", &[(0, "string", "string", false)]),
    (
        "strnatcasecmp",
        &[
            (0, "string1", "string", false),
            (1, "string2", "string", false),
        ],
    ),
    (
        "strnatcmp",
        &[
            (0, "string1", "string", false),
            (1, "string2", "string", false),
        ],
    ),
    (
        "strncasecmp",
        &[
            (0, "string1", "string", false),
            (1, "string2", "string", false),
        ],
    ),
    (
        "strncmp",
        &[
            (0, "string1", "string", false),
            (1, "string2", "string", false),
        ],
    ),
    (
        "strpbrk",
        &[
            (0, "string", "string", false),
            (1, "characters", "string", false),
        ],
    ),
    (
        "strpos",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
        ],
    ),
    (
        "strptime",
        &[
            (0, "timestamp", "string", false),
            (1, "format", "string", false),
        ],
    ),
    (
        "strrchr",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
        ],
    ),
    ("strrev", &[(0, "string", "string", false)]),
    (
        "strripos",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
        ],
    ),
    (
        "strrpos",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
        ],
    ),
    (
        "strspn",
        &[
            (0, "string", "string", false),
            (1, "characters", "string", false),
        ],
    ),
    (
        "strstr",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
        ],
    ),
    (
        "strtok",
        &[
            (0, "string", "string", false),
            (1, "token", "?string", false),
        ],
    ),
    ("strtolower", &[(0, "string", "string", false)]),
    ("strtotime", &[(0, "datetime", "string", false)]),
    ("strtoupper", &[(0, "string", "string", false)]),
    (
        "strtr",
        &[
            (0, "string", "string", false),
            (1, "from", "array|string", false),
            (2, "to", "?string", false),
        ],
    ),
    ("substr", &[(0, "string", "string", false)]),
    (
        "substr_compare",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
        ],
    ),
    (
        "substr_count",
        &[
            (0, "haystack", "string", false),
            (1, "needle", "string", false),
        ],
    ),
    (
        "substr_replace",
        &[
            (0, "string", "array|string", false),
            (1, "replace", "array|string", false),
        ],
    ),
    (
        "symlink",
        &[(0, "target", "string", false), (1, "link", "string", false)],
    ),
    ("syslog", &[(1, "message", "string", false)]),
    ("system", &[(0, "command", "string", false)]),
    (
        "tempnam",
        &[
            (0, "directory", "string", false),
            (1, "prefix", "string", false),
        ],
    ),
    ("test2", &[(0, "str", "string", false)]),
    ("textdomain", &[(0, "domain", "?string", false)]),
    ("tidy_get_opt_doc", &[(1, "option", "string", false)]),
    ("tidy_getopt", &[(1, "option", "string", false)]),
    (
        "tidy_parse_file",
        &[
            (0, "filename", "string", false),
            (1, "config", "array|string|null", false),
            (2, "encoding", "?string", false),
        ],
    ),
    (
        "tidy_parse_string",
        &[
            (0, "string", "string", false),
            (1, "config", "array|string|null", false),
            (2, "encoding", "?string", false),
        ],
    ),
    (
        "tidy_repair_file",
        &[
            (0, "filename", "string", false),
            (1, "config", "array|string|null", false),
            (2, "encoding", "?string", false),
        ],
    ),
    (
        "tidy_repair_string",
        &[
            (0, "string", "string", false),
            (1, "config", "array|string|null", false),
            (2, "encoding", "?string", false),
        ],
    ),
    (
        "timezone_identifiers_list",
        &[(1, "countryCode", "?string", false)],
    ),
    ("timezone_name_from_abbr", &[(0, "abbr", "string", false)]),
    ("timezone_open", &[(0, "timezone", "string", false)]),
    ("token_get_all", &[(0, "code", "string", false)]),
    ("touch", &[(0, "filename", "string", false)]),
    ("trait_exists", &[(0, "trait", "string", false)]),
    ("transliterator_create", &[(0, "id", "string", false)]),
    (
        "transliterator_create_from_rules",
        &[(0, "rules", "string", false)],
    ),
    (
        "transliterator_transliterate",
        &[(1, "string", "string", false)],
    ),
    ("trigger_error", &[(0, "message", "string", false)]),
    (
        "trim",
        &[
            (0, "string", "string", false),
            (1, "characters", "string", false),
        ],
    ),
    ("ucfirst", &[(0, "string", "string", false)]),
    (
        "ucwords",
        &[
            (0, "string", "string", false),
            (1, "separators", "string", false),
        ],
    ),
    ("uniqid", &[(0, "prefix", "string", false)]),
    ("unlink", &[(0, "filename", "string", false)]),
    (
        "unpack",
        &[
            (0, "format", "string", false),
            (1, "string", "string", false),
        ],
    ),
    ("unserialize", &[(0, "data", "string", false)]),
    ("urldecode", &[(0, "string", "string", false)]),
    ("urlencode", &[(0, "string", "string", false)]),
    ("user_error", &[(0, "message", "string", false)]),
    ("utf8_decode", &[(0, "string", "string", false)]),
    ("utf8_encode", &[(0, "string", "string", false)]),
    (
        "version_compare",
        &[
            (0, "version1", "string", false),
            (1, "version2", "string", false),
            (2, "operator", "?string", false),
        ],
    ),
    ("vfprintf", &[(1, "format", "string", false)]),
    ("vprintf", &[(0, "format", "string", false)]),
    ("vsprintf", &[(0, "format", "string", false)]),
    (
        "wordwrap",
        &[
            (0, "string", "string", false),
            (2, "break", "string", false),
        ],
    ),
    ("xml_parse", &[(1, "data", "string", false)]),
    ("xml_parse_into_struct", &[(1, "data", "string", false)]),
    ("xml_parser_create", &[(0, "encoding", "?string", false)]),
    (
        "xml_parser_create_ns",
        &[
            (0, "encoding", "?string", false),
            (1, "separator", "string", false),
        ],
    ),
    ("xmlwriter_open_uri", &[(0, "uri", "string", false)]),
    (
        "xmlwriter_set_indent_string",
        &[(1, "indentation", "string", false)],
    ),
    ("xmlwriter_start_attribute", &[(1, "name", "string", false)]),
    (
        "xmlwriter_start_attribute_ns",
        &[
            (1, "prefix", "?string", false),
            (2, "name", "string", false),
            (3, "namespace", "?string", false),
        ],
    ),
    (
        "xmlwriter_start_document",
        &[
            (1, "version", "?string", false),
            (2, "encoding", "?string", false),
            (3, "standalone", "?string", false),
        ],
    ),
    (
        "xmlwriter_start_dtd",
        &[
            (1, "qualifiedName", "string", false),
            (2, "publicId", "?string", false),
            (3, "systemId", "?string", false),
        ],
    ),
    (
        "xmlwriter_start_dtd_attlist",
        &[(1, "name", "string", false)],
    ),
    (
        "xmlwriter_start_dtd_element",
        &[(1, "qualifiedName", "string", false)],
    ),
    (
        "xmlwriter_start_dtd_entity",
        &[(1, "name", "string", false)],
    ),
    ("xmlwriter_start_element", &[(1, "name", "string", false)]),
    (
        "xmlwriter_start_element_ns",
        &[
            (1, "prefix", "?string", false),
            (2, "name", "string", false),
            (3, "namespace", "?string", false),
        ],
    ),
    ("xmlwriter_start_pi", &[(1, "target", "string", false)]),
    ("xmlwriter_text", &[(1, "content", "string", false)]),
    (
        "xmlwriter_write_attribute",
        &[(1, "name", "string", false), (2, "value", "string", false)],
    ),
    (
        "xmlwriter_write_attribute_ns",
        &[
            (1, "prefix", "?string", false),
            (2, "name", "string", false),
            (3, "namespace", "?string", false),
            (4, "value", "string", false),
        ],
    ),
    ("xmlwriter_write_cdata", &[(1, "content", "string", false)]),
    (
        "xmlwriter_write_comment",
        &[(1, "content", "string", false)],
    ),
    (
        "xmlwriter_write_dtd",
        &[
            (1, "name", "string", false),
            (2, "publicId", "?string", false),
            (3, "systemId", "?string", false),
            (4, "content", "?string", false),
        ],
    ),
    (
        "xmlwriter_write_dtd_attlist",
        &[
            (1, "name", "string", false),
            (2, "content", "string", false),
        ],
    ),
    (
        "xmlwriter_write_dtd_element",
        &[
            (1, "name", "string", false),
            (2, "content", "string", false),
        ],
    ),
    (
        "xmlwriter_write_dtd_entity",
        &[
            (1, "name", "string", false),
            (2, "content", "string", false),
            (4, "publicId", "?string", false),
            (5, "systemId", "?string", false),
            (6, "notationData", "?string", false),
        ],
    ),
    (
        "xmlwriter_write_element",
        &[
            (1, "name", "string", false),
            (2, "content", "?string", false),
        ],
    ),
    (
        "xmlwriter_write_element_ns",
        &[
            (1, "prefix", "?string", false),
            (2, "name", "string", false),
            (3, "namespace", "?string", false),
            (4, "content", "?string", false),
        ],
    ),
    (
        "xmlwriter_write_pi",
        &[
            (1, "target", "string", false),
            (2, "content", "string", false),
        ],
    ),
    ("xmlwriter_write_raw", &[(1, "content", "string", false)]),
    ("zip_entry_open", &[(2, "mode", "string", false)]),
    ("zip_open", &[(0, "filename", "string", false)]),
    ("zlib_decode", &[(0, "data", "string", false)]),
    ("zlib_encode", &[(0, "data", "string", false)]),
];

/// The by-value `string`-accepting parameters of builtin `name` (lowercase):
/// php's zpp converts a `Stringable` object passed there with `__toString()`
/// and rejects any other object with a TypeError.
pub fn string_params(name: &str) -> &'static [(u8, &'static str, &'static str, bool)] {
    match STRING_PARAMS.binary_search_by(|(n, _)| (*n).cmp(name)) {
        Ok(i) => STRING_PARAMS[i].1,
        Err(_) => &[],
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sorted_and_found() {
        assert!(super::STRING_PARAMS.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(super::string_params("strlen")[0].1, "string");
        assert!(super::string_params("var_dump").is_empty());
    }
}
