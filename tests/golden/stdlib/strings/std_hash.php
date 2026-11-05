<?php
echo md5("hello world"), "\n";
echo sha1("hello world"), "\n";
echo hash("sha256", "abc"), "\n";
echo crc32("phrust"), "\n";
echo base64_encode("Hello, World!"), "\n";
echo base64_decode("SGVsbG8="), "\n";
echo bin2hex("ABC"), "\n";
var_dump(hex2bin("414243"));
// md5/sha1/crc32 vectors, hex and raw, around the 64-byte block's padding.
$vectors = ["", "abc", "message digest", "The quick brown fox jumps over the lazy dog",
    "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"];
foreach ([55, 56, 63, 64, 65] as $n) {
    $vectors[] = str_repeat("a", $n);
}
foreach ($vectors as $s) {
    echo strlen($s), " ", md5($s), " ", bin2hex(md5($s, true)), "\n";
    echo strlen($s), " ", sha1($s), " ", bin2hex(sha1($s, true)), "\n";
    echo strlen($s), " ", crc32($s), "\n";
}
var_dump(strlen(md5("abc", true)), strlen(sha1("abc", true)));
