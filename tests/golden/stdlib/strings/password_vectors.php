<?php
// password_*: verification of fixed hashes from every crypt() algorithm and
// both argon2 variants (including strings libargon2 rejects), get_info,
// needs_rehash, option validation and round trips through password_hash.
$pw = 'correct horse';
$hashes = [
    '$argon2i$v=19$m=1024,t=1,p=1$bzFJRVJleTJ3cmluQkQvTQ$3vsDl7bnsMKoCMRuXilqZL/162WwSKDKb9va+ODsdak',
    '$argon2i$v=19$m=2048,t=2,p=2$Y09IMi5hZ0JVQzNJc0RFTg$NEQJaUXDchjfFCx2wBBfHMDa5UgdpgKlVCQQiwoml0I',
    '$argon2i$v=19$m=65536,t=4,p=1$MzFOWmtKalIwQ3pGczRIVw$woAedWDVnqVNEb/6XjSMM6Z6AV2M0YjGNFhv4FIJZhU',
    '$argon2id$v=19$m=1024,t=1,p=1$NHJKOEV5OS4uTDdJWlBjaA$rmZbdETNeCsH/qlAKklgMueIeOT6XYMvcocyy9oYrUQ',
    '$argon2id$v=19$m=2048,t=2,p=2$bXJNQXpRcnZYdzZ0WEZsag$T8Vro0u8MWBdtyafFY3P4cNc9SSKw/IhcPFNeUNyE2A',
    '$argon2id$v=19$m=65536,t=4,p=1$VVN1dnFtM0ZTL3UwbGh2MA$PRd0tpml2H+3nT17DXRNIZHIH20d+fnv1h2nofTFl7A',
    '$2y$04$Kq6bxajkoNntZky0MSyfsOhVtJE1CVdu/E9g4ykvqjrhXzITRZcny',
    '$1$abcdefgh$y6iHhJNbuC0xpbk0w9pm80',
    '$5$rounds=1000$abcdefgh$ue35C0ITbth1b5zVUjrjm2mMmsmMKiodEeH2UV/Hq10',
    '$6$abcdefgh$yIZAF3gQPvtKZO/9qOJKffAKKbtS3ef3qmwyugk4uWVjX8YZf/GV3A8SkFxEPY0T56CcilGrHKLffBsp6dLMG.',
    'abhfCpXqd4GrI',
    '_J9..abcdtIvPUrZYa6w',
    '$2a$04$abcdefghijklmnopqrstuujydOTSfIH/d5oUHpsygqV5X9xJLQc6e',
    '$2x$04$abcdefghijklmnopqrstuujydOTSfIH/d5oUHpsygqV5X9xJLQc6e',
    // libargon2 is stricter than a PHC parser: no version, reordered or
    // missing parameters, the wrong variant, a truncated digest.
    '$argon2i$m=1024,t=1,p=1$bzFJRVJleTJ3cmluQkQvTQ$3vsDl7bnsMKoCMRuXilqZL/162WwSKDKb9va+ODsdak',
    '$argon2i$v=19$t=1,m=1024,p=1$bzFJRVJleTJ3cmluQkQvTQ$3vsDl7bnsMKoCMRuXilqZL/162WwSKDKb9va+ODsdak',
    '$argon2i$v=19$m=1024,t=1$bzFJRVJleTJ3cmluQkQvTQ$3vsDl7bnsMKoCMRuXilqZL/162WwSKDKb9va+ODsdak',
    '$argon2id$v=19$m=1024,t=1,p=1$bzFJRVJleTJ3cmluQkQvTQ$3vsDl7bnsMKoCMRuXilqZL/162WwSKDKb9va+ODsdak',
    '$argon2d$v=19$m=1024,t=1,p=1$bzFJRVJleTJ3cmluQkQvTQ$3vsDl7bnsMKoCMRuXilqZL/162WwSKDKb9va+ODsdak',
    '$argon2i$v=16$m=1024,t=1,p=1$bzFJRVJleTJ3cmluQkQvTQ$3vsDl7bnsMKoCMRuXilqZL/162WwSKDKb9va+ODsdak',
    '$argon2i$v=19$m=1024,t=1,p=1$bzFJRVJleTJ3cmluQkQvTQ$3vsDl7bnsMKoCMRuXilqZL/162WwSKDKb9va+ODsda',
    '$argon2i$v=19$m=1024,t=1,p=1$bzFJRVJleTJ3cmluQkQvTQ$3vsDl7bnsMKoCMRuXilqZL/162WwSKDKb9va+ODsdak=',
    '$argon2i$v=19$m=1024,t=1,p=1$bzFJRVJleTJ3cmluQkQvTQ$3vsDl7bnsMKoCMRuXilqZL/162WwSKDKb9va+ODsdak' . "\0junk",
    '$argon2i$v=19$m=1024,t=1,p=1$$3vsDl7bnsMKoCMRuXilqZL/162WwSKDKb9va+ODsdak',
    '$argon2i$',
    // bcrypt-shaped strings that aren't valid "2y" hashes
    '$2y$04$Kq6bxajkoNntZky0MSyfsOhVtJE1CVdu/E9g4ykvqjrhXzITRZcn',
    '$2y$04$Kq6bxajkoNntZky0MSyfsOhVtJE1CVdu/E9g4ykvqjrhXzITRZcnyX',
    'x2y$04$Kq6bxajkoNntZky0MSyfsOhVtJE1CVdu/E9g4ykvqjrhXzITRZcny',
    '$2y$04$Kq6bxajkoNntZky0MSyfsOhVtJE1CVdu/E9g4ykvqjrhXzITRZcn' . "\0",
    '$2y$03$Kq6bxajkoNntZky0MSyfsOhVtJE1CVdu/E9g4ykvqjrhXzITRZcny',
    '$2y$+4$Kq6bxajkoNntZky0MSyfsOhVtJE1CVdu/E9g4ykvqjrhXzITRZcny',
    // crypt() failure strings and short hashes
    '*0',
    '*1',
    'ab',
    'abhfCpXqd4Gr',
    '',
    '$',
    '$$$',
];
foreach ($hashes as $i => $h) {
    echo "#$i ", json_encode([
        password_verify($pw, $h),
        password_verify('wrong', $h),
        password_get_info($h),
        password_needs_rehash($h, PASSWORD_DEFAULT),
        password_needs_rehash($h, PASSWORD_BCRYPT, ['cost' => 4]),
        password_needs_rehash($h, PASSWORD_ARGON2I, ['memory_cost' => 1024, 'time_cost' => 1, 'threads' => 1]),
        password_needs_rehash($h, PASSWORD_ARGON2ID),
    ]), "\n";
}

// bcrypt reads the password as a C string and only its first 72 bytes.
$h = '$2y$04$Kq6bxajkoNntZky0MSyfsOhVtJE1CVdu/E9g4ykvqjrhXzITRZcny';
var_dump(password_verify("correct horse\0anything", $h));
$long = str_repeat('a', 72);
$h = crypt($long, '$2y$04$abcdefghijklmnopqrstuu');
var_dump(password_verify($long . 'tail', $h), password_verify(substr($long, 0, 71), $h));
// argon2 hashes the whole password.
$h = $hashes[3];
var_dump(password_verify("correct horse\0", $h));

// needs_rehash reads argon2 parameters with sscanf; what it can't read is 0.
foreach ([
    '$argon2id$v=19$m=65536,t=4,p=1$x$y',
    '$argon2id$v=19$m=65536,t=4',
    '$argon2id$v=19$m=65536',
    '$argon2id$v=19$m= 65536,t=+4,p=1$',
    '$argon2id$',
    '$argon2id$v=19$m=99999999999999999999999,t=4,p=1$',
] as $h) {
    echo json_encode([password_needs_rehash($h, PASSWORD_ARGON2ID), password_get_info($h)]), "\n";
}
var_dump(password_needs_rehash('$2y$12$Kq6bxajkoNntZky0MSyfsOhVtJE1CVdu/E9g4ykvqjrhXzITRZcny', PASSWORD_BCRYPT));
var_dump(password_needs_rehash('$2y$12$Kq6bxajkoNntZky0MSyfsOhVtJE1CVdu/E9g4ykvqjrhXzITRZcny', PASSWORD_BCRYPT, ['cost' => '12']));
var_dump(password_needs_rehash('$2y$04$Kq6bxajkoNntZky0MSyfsOhVtJE1CVdu/E9g4ykvqjrhXzITRZcny', 'unknown'));
var_dump(password_needs_rehash('$2y$04$Kq6bxajkoNntZky0MSyfsOhVtJE1CVdu/E9g4ykvqjrhXzITRZcny', 99));
var_dump(password_needs_rehash('$2y$04$Kq6bxajkoNntZky0MSyfsOhVtJE1CVdu/E9g4ykvqjrhXzITRZcny', 1, ['cost' => 4]));
var_dump(password_needs_rehash('$2y$04$Kq6bxajkoNntZky0MSyfsOhVtJE1CVdu/E9g4ykvqjrhXzITRZcny', null, ['cost' => 4]));
var_dump(password_needs_rehash('$2y$04$Kq6bxajkoNntZky0MSyfsOhVtJE1CVdu/E9g4ykvqjrhXzITRZcny', true, ['cost' => 4]));

echo json_encode(password_algos()), "\n";
foreach (get_defined_constants() as $name => $value) {
    if (str_starts_with($name, 'PASSWORD_')) {
        echo $name, '=', var_export($value, true), "\n";
    }
}

// Errors, in php's order of checks.
$errors = [
    ['pw', 'nope', []],
    ['pw', 4, []],
    ['pw', '2y', ['cost' => 3]],
    ['pw', '2y', ['cost' => 32]],
    ['pw', '2y', ['cost' => 'abc']],
    ["p\0w", '2y', []],
    ["p\0w", '2y', ['cost' => 3]],
    ['pw', 'argon2i', ['memory_cost' => 7]],
    ['pw', 'argon2i', ['memory_cost' => -1]],
    ['pw', 'argon2i', ['memory_cost' => 4294967296]],
    ['pw', 'argon2id', ['time_cost' => 0]],
    ['pw', 'argon2id', ['time_cost' => -5]],
    ['pw', 'argon2id', ['threads' => 0]],
    ['pw', 'argon2id', ['threads' => 16777216]],
    ['pw', 'argon2id', ['threads' => -1]],
    ['pw', 'argon2id', ['memory_cost' => 8, 'time_cost' => 1, 'threads' => 2]],
    ['pw', 'argon2i', ['memory_cost' => 7, 'time_cost' => 0, 'threads' => 0]],
];
foreach ($errors as [$p, $algo, $opts]) {
    try {
        password_hash($p, $algo, $opts);
        echo "no error\n";
    } catch (Throwable $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}
try {
    password_hash('pw', []);
} catch (TypeError $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}

// Round trips: the salt is random, so print what doesn't depend on it.
set_error_handler(function ($no, $str) {
    echo "warning: $str\n";
    return true;
});
foreach ([
    [PASSWORD_DEFAULT, ['cost' => 4]],
    [PASSWORD_BCRYPT, ['cost' => 5, 'salt' => 'ignored']],
    [1, ['cost' => 4]],
    [null, ['cost' => '04']],
    [PASSWORD_ARGON2I, ['memory_cost' => 1024, 'time_cost' => 1, 'threads' => 1]],
    [PASSWORD_ARGON2ID, ['memory_cost' => 2048, 'time_cost' => 2, 'threads' => 3]],
    [2, ['memory_cost' => 1024, 'time_cost' => 1, 'salt' => null]],
    [3, ['memory_cost' => 1024, 'time_cost' => 1]],
] as [$algo, $opts]) {
    $h = password_hash($pw, $algo, $opts);
    $info = password_get_info($h);
    $salt = substr($h, strrpos($h, '$', -32) + 1, 22);
    echo json_encode([
        strlen($h),
        preg_replace('/\$[^$]*\$[^$]*$/', '', $h),
        $info,
        password_verify($pw, $h),
        password_verify('wrong', $h),
        password_needs_rehash($h, $algo, $opts),
        password_needs_rehash($h, $algo),
    ]), "\n";
}
// The argon2 salt is 16 characters of php's modified base64.
$h = password_hash($pw, PASSWORD_ARGON2ID, ['memory_cost' => 1024, 'time_cost' => 1]);
$salt = base64_decode(explode('$', $h)[4]);
var_dump(strlen($salt), (bool) preg_match('#^[A-Za-z0-9./]{16}$#', $salt));
$h = password_hash($pw, PASSWORD_BCRYPT, ['cost' => 4]);
var_dump((bool) preg_match('#^\$2y\$04\$[A-Za-z0-9./]{53}$#', $h));
