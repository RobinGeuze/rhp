<?php
// hash_pbkdf2 over every HMAC-capable algorithm: default length, a multi-block
// hex length, a raw multi-block key, several iterations, a password longer than
// the algorithm's block size, an empty password and salt.
$long = str_repeat("pass-word!", 30);
foreach (hash_hmac_algos() as $algo) {
    echo $algo, "\n";
    echo "  ", hash_pbkdf2($algo, "password", "salt", 1), "\n";
    echo "  ", hash_pbkdf2($algo, "password", "NaCl", 3, 101), "\n";
    echo "  ", bin2hex(hash_pbkdf2($algo, $long, "salt", 2, 70, true)), "\n";
    echo "  ", hash_pbkdf2($algo, "", "", 5, 7), "\n";
}
// a large key: only its length and last hex digit are printed
$h = hash_pbkdf2("md5", "password", "salt", 1, 200001);
echo strlen($h), " ", substr($h, -9), "\n";
$h = hash_pbkdf2("sha512", "password", "salt", 2, 70000, true);
echo strlen($h), " ", bin2hex(substr($h, -9)), "\n";
