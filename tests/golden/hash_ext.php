<?php
foreach (["md5","sha256","sha3-256","ripemd160","whirlpool","crc32","crc32b","tiger192,4","snefru","haval256,5","xxh3","murmur3f"] as $a) {
  echo str_pad($a, 12), hash($a, "The quick brown fox"), "\n";
}
echo hash_hmac("sha256", "msg", "key"), "\n";
echo hash_pbkdf2("sha256", "password", "salt", 1000, 32), "\n";
var_dump(hash_equals("abc", "abc"), hash_equals("abc", "abd"));
