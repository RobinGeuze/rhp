<?php
echo md5("hello"), "\n";
echo sha1("hello"), "\n";
echo hash('sha256', "hello"), "\n";
echo crc32("hello"), "\n";
echo base64_encode("Hello, World!"), "\n";
echo base64_decode("SGVsbG8="), "\n";
echo bin2hex("ABC"), "\n";
echo hex2bin("414243"), "\n";
echo strrev(dechex(crc32("test"))), "\n";
echo substr(md5("a"), 0, 8), "\n";
echo hash('md5', ''), "\n";
echo urlencode("a b&c=d"), "\n";
echo rawurlencode("a b&c"), "\n";
echo urldecode("a%20b%26c"), "\n";
echo htmlentities("<a href=\"x\">é</a>"), "\n";
echo html_entity_decode("&lt;b&gt;&amp;"), "\n";
echo ord("A"), " ", chr(66), "\n";
echo str_rot13("Hello"), "\n";
