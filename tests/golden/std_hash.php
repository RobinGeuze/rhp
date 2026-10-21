<?php
echo md5("hello world"), "\n";
echo sha1("hello world"), "\n";
echo hash("sha256", "abc"), "\n";
echo crc32("phrust"), "\n";
echo base64_encode("Hello, World!"), "\n";
echo base64_decode("SGVsbG8="), "\n";
echo bin2hex("ABC"), "\n";
var_dump(hex2bin("414243"));
