<?php
// FILTER_VALIDATE_EMAIL runs php's own regexes through PCRE2 (regexp0 with
// FILTER_FLAG_EMAIL_UNICODE). Filtering leaves preg_last_error() alone.
$in = ['a@b.com', 'é@b.de', 'user@exämple.com', 'a@b', '"quoted"@example.com', '"a\"b"@x.com', 'user@[IPv6:2001:db8::1]',
  'user@[1.2.3.4]', 'a@b.c1', 'a@1b.bc', 'x@xn--bcher-kva.ch', 'a@b-.com', 'a..b@c.com', '.a@b.com', 'a.@b.com',
  "a@b.com\n", "\xff@b.com", "ü@ü.de", 'a@[IPv6:::1]', 'a@[IPv6:1:2:3:4:5:6:7:8]', 'a@[IPv6:1::2:3.4.5.6]',
  str_repeat('a', 64).'@b.com', str_repeat('a', 65).'@b.com', 'a@'.str_repeat('b', 63).'.com', 'a@'.str_repeat('b', 64).'.com',
  str_repeat('a', 60).'@'.str_repeat('b.', 130).'com', 'A@B.COM', 'a@B.Com', "\"\x01\"@x.com", '"\\\\"@x.com',
  'üñî@çø.de', 'ab@cd', 'a@b.c-d', 'a@b.c--d', 'a@b.xn--p1ai', '1@2.3', 'test@localhost.localdomain', ''];
foreach ($in as $s) {
  echo json_encode(substr($s, 0, 40)), ' ', strlen($s), ' ',
    var_export(filter_var($s, FILTER_VALIDATE_EMAIL) !== false, true), ' ',
    var_export(filter_var($s, FILTER_VALIDATE_EMAIL, FILTER_FLAG_EMAIL_UNICODE) !== false, true), "\n";
}
var_dump(filter_var(str_repeat('a', 300).'@b.com', FILTER_VALIDATE_EMAIL));
var_dump(filter_var('bad', FILTER_VALIDATE_EMAIL, ['options' => ['default' => 'x']]));
filter_var("\xff@b.com", FILTER_VALIDATE_EMAIL, FILTER_FLAG_EMAIL_UNICODE); var_dump(preg_last_error());
preg_match('/a/u', "\xff"); var_dump(preg_last_error());
filter_var('a@b.com', FILTER_VALIDATE_EMAIL); var_dump(preg_last_error());
