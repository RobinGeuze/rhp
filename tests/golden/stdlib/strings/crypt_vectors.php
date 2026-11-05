<?php
// crypt() on every algorithm of php's own crypt C: known vectors, edge
// salts, C-string truncation and the "*0" / "*1" failure strings.
$cases = [
    // standard DES
    ['rasmuslerdorf', 'rl'],
    ['rasmuslerdorf', 'rl.3StKT.4T8M'],
    ['', 'ab'],
    ['password', './'],
    ['password', 'zZ'],
    ['12345678extra', 'ab'],
    ['x', 'a'],
    ['x', 'a:'],
    ['x', "a\n"],
    ['x', "\x01\x02"],
    ['x', '~~'],
    // extended DES
    ['rasmuslerdorf', '_J9..rasm'],
    ['', '_J9..SDSD'],
    ['726 even', '_K9..Salt'],
    ['a long password that is hashed in 8-byte steps', '_J9..rasm'],
    ['x', '_J9..'],
    ['x', '_........'],
    ['x', '_/!......'],
    ['x', '_/......!'],
    ['x', '_'],
    // MD5
    ['rasmuslerdorf', '$1$rasmusle$'],
    ['', '$1$'],
    ['pw', '$1$saltsaltsalt$'],
    ['pw', '$1$a$b'],
    ['pw', '$1$a'],
    ['pw', '$1$' . str_repeat('x', 200)],
    [str_repeat('p', 100), '$1$s$'],
    ["\xff\xfe\x80", '$1$hi$'],
    // blowfish
    ['rasmuslerdorf', '$2a$07$rasmuslerd............'],
    ['U*U', '$2a$05$CCCCCCCCCCCCCCCCCCCCC.'],
    ['U*U*', '$2a$05$CCCCCCCCCCCCCCCCCCCCC.'],
    ['', '$2a$05$CCCCCCCCCCCCCCCCCCCCC.'],
    ["\xa3", '$2x$05$/OK.fbVrR/bpIqNJ5ianF.'],
    ["\xa3", '$2y$05$/OK.fbVrR/bpIqNJ5ianF.'],
    ["\xa3", '$2a$05$/OK.fbVrR/bpIqNJ5ianF.'],
    ["\xff\xa3" . '345', '$2x$05$/OK.fbVrR/bpIqNJ5ianF.'],
    ["\xff\xa3" . '345', '$2a$05$/OK.fbVrR/bpIqNJ5ianF.'],
    ["\xff\xa3" . '345', '$2y$05$/OK.fbVrR/bpIqNJ5ianF.'],
    ['0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789chars after 72 are ignored', '$2a$05$abcdefghijklmnopqrstuu'],
    ['0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789chars after 72 are ignored as usual', '$2a$05$abcdefghijklmnopqrstuu'],
    ['pw', '$2b$04$abcdefghijklmnopqrstuu'],
    ['pw', '$2y$04$abcdefghijklmnopqrstuv'],
    ['pw', '$2y$04$abcdefghijklmnopqrstuuTRAILING'],
    ['pw', '$2c$04$abcdefghijklmnopqrstuu'],
    ['pw', '$2y$03$abcdefghijklmnopqrstuu'],
    ['pw', '$2y$32$abcdefghijklmnopqrstuu'],
    ['pw', '$2y$4$abcdefghijklmnopqrstuuu'],
    ['pw', '$2y$04$abcdefghijklmnopqrstu'],
    ['pw', '$2y$04$abcdefghij$lmnopqrstuu'],
    ['pw', '$2y$04$'],
    ['pw', '$2$04$abcdefghijklmnopqrstuu'],
    // SHA-256
    ['rasmuslerdorf', '$5$rounds=5000$usesomesillystri$'],
    ['Hello world!', '$5$saltstring'],
    ['Hello world!', '$5$rounds=10000$saltstringsaltstring'],
    ['we have a short salt string but not a short password', '$5$rounds=77777$short'],
    ['a short string', '$5$rounds=123456$asaltof16chars..'],
    ['the minimum number is still observed', '$5$rounds=1000$roundstoolow'],
    ['', '$5$rounds=1000$$'],
    ['x', '$5$rounds=999$abc$'],
    ['x', '$5$rounds=1000000000$abc$'],
    ['x', '$5$rounds=abc$salt$'],
    ['x', '$5$rounds=1000'],
    ['x', '$5$rounds=-1$salt$'],
    ['x', '$5$rounds= 1000$salt$'],
    ['x', '$5$'],
    ['x', '$5$' . str_repeat('s', 200)],
    // SHA-512
    ['rasmuslerdorf', '$6$rounds=5000$usesomesillystri$'],
    ['Hello world!', '$6$saltstring'],
    ['Hello world!', '$6$rounds=10000$saltstringsaltstring'],
    ['a very much longer text to encrypt.  This one even stretches over morethan one line.', '$6$rounds=1400$anotherlongsaltstring'],
    ['x', '$6$sa!t#{}$'],
    ['x', '$6$rounds=1000$'],
    ['x', '$6$rounds=999$abc$'],
    ['x', '$6$'],
    // failures and dispatch
    ['x', '*0'],
    ['x', '*1'],
    ['x', '*0trailing'],
    ['x', '*'],
    ['x', ''],
    ['x', '$'],
    ['x', '$9$abc'],
    ['x', '$:#'],
    ['x', "$:5zd$01\n"],
    // C strings: both stop at the first NUL
    ["a\0b", 'ab'],
    ['a', 'ab'],
    ['pw', "ab\0cd"],
    ['pw', "\0ab"],
    ['pw', "*\x000"],
    ["pw\0", '$6$salt$'],
];
foreach ($cases as $i => [$pw, $salt]) {
    echo $i, ' ', crypt($pw, $salt), "\n";
}

// The hash of a hash, the way verification works.
$h = crypt('secret', '$6$rounds=2000$pepper$');
var_dump(crypt('secret', $h) === $h, crypt('Secret', $h) === $h);
$h = crypt('secret', '$2y$04$0123456789abcdefghijkl');
var_dump(crypt('secret', $h) === $h, crypt('secret!', $h) === $h);

foreach (['CRYPT_SALT_LENGTH', 'CRYPT_STD_DES', 'CRYPT_EXT_DES', 'CRYPT_MD5',
          'CRYPT_BLOWFISH', 'CRYPT_SHA256', 'CRYPT_SHA512'] as $c) {
    echo $c, '=', constant($c), "\n";
}

try {
    crypt('x');
} catch (ArgumentCountError $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
try {
    crypt([], 'ab');
} catch (TypeError $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
echo crypt(12345, 'ab'), "\n";
