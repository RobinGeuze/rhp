<?php
// iconv_* failure paths. php emits E_NOTICE/E_WARNING here (rhp routes them to
// its diagnostic sink, which the engine does not display yet — placeholder
// #32), so each call is @-silenced and only the return values are compared.
var_dump(@iconv_strlen("h\xe9llo"), @iconv_strlen("h\xc3"), @iconv_strlen("abc", "bogus"), @iconv_strlen("abc", str_repeat("x", 64)));
var_dump(@iconv_substr("h\xe9llo", 1), @iconv_substr("abc", 1, 1, "bogus"));
// libiconv's one-character lookahead: an error right after the last decoded
// character poisons the result; a forward match that ends earlier survives.
var_dump(@iconv_strrpos("a\xffbc", "a"), @iconv_strrpos("abcdef\xff", "a"), @iconv_strrpos("\xffab", "a"), @iconv_strrpos("abc", "\xff"));
var_dump(@iconv_strpos("ab\xff", "b"), @iconv_strpos("ab\xffcd", "b", 0, "ASCII"), @iconv_strpos("ab\xc3", "b"), @iconv_strpos("ab\xc3", "a"));
var_dump(@iconv_strpos("abc\xff", "a"), @iconv_strpos("abc\xff", "b"), @iconv_strpos("abc\xff", "c"), @iconv_strpos("abc\xff", "z"), @iconv_strpos("abc\xff", "a", 10));
var_dump(@iconv_strpos("abc", "b", 0, "bogus"), @iconv_strpos("abc", "b", -1, "bogus"), @iconv_strpos("h\xe9llo", "l", -1));
try { iconv_strpos("\xff", "a", 3); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(@iconv_mime_decode("=?UTF-8?X?abc?="), @iconv_mime_decode("=?FOO?Q?abc?= x"), @iconv_mime_decode("", 0, "bogus"), @iconv_mime_decode("=?UTF-8?Q?\xff?="), @iconv_mime_decode("=?UTF-8?Q?=ZZ?="), @iconv_mime_decode("=?UTF-8?B?***?="));
