<?php
// PHP_MHASH_BC: the mhash*() functions and their deprecated MHASH_* constants.
var_dump(MHASH_CRC32, MHASH_XXH128, defined('MHASH_SNEFRU256'));
var_dump(mhash_count());
foreach ([0, 1, 3, 4, 7, 26, 33, 41, 42, -1] as $n) {
    var_dump(mhash_get_hash_name($n), mhash_get_block_size($n));
}
var_dump(bin2hex(mhash(1, 'abc')), bin2hex(mhash(2, 'abc', 'key')), mhash(4, 'x'));
try { mhash(0, 'abc', 'key'); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
var_dump(bin2hex(mhash_keygen_s2k(1, 'pass', 'saltsaltsalt', 20)));
var_dump(bin2hex(mhash_keygen_s2k(0, 'pass', 'ab', 9)));
try { mhash_keygen_s2k(1, 'p', 's', 0); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
