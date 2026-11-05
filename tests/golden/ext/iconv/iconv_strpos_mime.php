<?php
// iconv_strlen / iconv_substr / iconv_strpos / iconv_strrpos / iconv_mime_decode
// (warning-free cases; the failure paths are in iconv_error_returns.php).
var_dump(iconv_strlen("aé"), iconv_strlen(""), iconv_strlen("ab", "UTF-16LE"), iconv_strlen("\xe9t\xe9", "ISO-8859-1"), iconv_strlen("x", null));
var_dump(iconv_substr("aéb", 1, 1), iconv_substr("abc", 5), iconv_substr("abc", 3), iconv_substr("abcdef", -2), iconv_substr("abcdef", 1, -2), iconv_substr("abcdef", -10, 2), iconv_substr("abcdef", 2, -10), iconv_substr("añb", 1, null, "UTF-8"), bin2hex(iconv_substr("\xe9t\xe9", 1, 2, "latin1")));
var_dump(iconv_strpos("aéb", "b"), iconv_strpos("abc", "", 0), iconv_strpos("aéb", "é", 1, "UTF-8"), iconv_strpos("abc", "c", -1), iconv_strpos("abc", "a", -1));
var_dump(iconv_strpos("aaab", "aab"), iconv_strpos("abcabd", "abd"), iconv_strpos("ababac", "abac"), iconv_strpos("aabaabaaab", "aaab"), iconv_strpos("xyz", "yzx"), iconv_strpos("abc", "abc", 3), iconv_strpos("abc", "c", 3));
var_dump(iconv_strrpos("abab", "b"), iconv_strrpos("aaaa", "aa"), iconv_strrpos("abc", ""), iconv_strrpos("éaé", "é"), iconv_strrpos("abc", "d"), iconv_strrpos("a\x00b\x00", "b\x00", "UTF-16LE"));
try { iconv_strpos("abc", "a", 4); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { iconv_strpos("abc", "a", -4); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(iconv_strpos("abc", "", 4));
var_dump(
    iconv_mime_decode("Subject: =?UTF-8?B?UHLDvGZ1bmcgUHLDvGZ1bmc=?="),
    iconv_mime_decode("=?ISO-8859-1?Q?Andr=E9?= Pirard"),
    iconv_mime_decode("a =?UTF-8?Q?x_y?= =?UTF-8?Q?z?= b"),
    iconv_mime_decode("=?UTF-8?X?abc?=", ICONV_MIME_DECODE_CONTINUE_ON_ERROR),
    iconv_mime_decode("=?FOO?Q?abc?= x", 2),
    iconv_mime_decode("plain\r\n folded"),
    bin2hex(iconv_mime_decode("=?UTF-8?B?w6k=?=", 0, "ISO-8859-1")),
    iconv_mime_decode("=?utf-8?q?a=3Db?=x", 1),
    iconv_mime_decode("=?utf-8?q?a=3Db?=x", 0),
    iconv_mime_decode("abc =?", 2),
    iconv_mime_decode("x=?utf-8*en?q?hi?="),
    iconv_mime_decode("A\r\nB: c"),
    iconv_mime_decode("=?UTF-8?B?SGVs bG8=?="),
    iconv_mime_decode("one\ttwo  three"),
    iconv_mime_decode("=?UTF-8?Q?soft=\r\n_break?="),
    iconv_mime_decode("bare\rcr"),
    ICONV_MIME_DECODE_STRICT,
    ICONV_MIME_DECODE_CONTINUE_ON_ERROR
);
