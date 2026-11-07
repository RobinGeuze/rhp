<?php
// A phar keeps only its manifest in memory: an entry is read from the
// archive file when it is used, so a large archive doesn't count against
// memory_limit (PHPStan's 30 MB phpstan.phar).

// A native phar with a SHA-1 signature: stub, manifest, data, signature.
function build_phar(string $alias, array $files): string {
    $stub = "<?php __HALT_COMPILER(); ?>\r\n";
    $entries = '';
    $data = '';
    foreach ($files as $name => $body) {
        $entries .= pack('V', strlen($name)) . $name
            . pack('VVVVVV', strlen($body), 1700000000, strlen($body), crc32($body), 0644, 0);
        $data .= $body;
    }
    $manifest = pack('V', count($files)) . "\x11\x10" . pack('V', 0x10000)
        . pack('V', strlen($alias)) . $alias . pack('V', 0) . $entries;
    $phar = $stub . pack('V', strlen($manifest)) . $manifest . $data;
    return $phar . sha1($phar, true) . pack('V', 2) . 'GBMB';
}

function big(): string {
    return str_repeat(md5('rhp'), (4 << 20) >> 5);
}

$dir = sys_get_temp_dir() . '/rhp_phar_on_demand_' . bin2hex(random_bytes(6));
mkdir($dir);
try {
    $file = "$dir/ondemand.phar";
    file_put_contents($file, build_phar('ondemand.phar', [
        'big.txt' => big(),
        'lib.php' => '<?php return "lib " . basename(__FILE__);',
    ]));

    // opened by path
    $before = memory_get_usage();
    var_dump(file_exists("phar://$file/big.txt"));
    var_dump(memory_get_usage() - $before < (1 << 20));
    var_dump(filesize("phar://$file/big.txt"));
    var_dump(include "phar://$file/lib.php");
    var_dump(file_get_contents("phar://$file/big.txt") === big());
    var_dump(memory_get_usage() - $before < (1 << 20));

    // opened by alias
    $file2 = "$dir/alias.phar";
    file_put_contents($file2, build_phar('alias.phar', ['big.txt' => big(), 'a.txt' => 'alias entry']));
    $before = memory_get_usage();
    var_dump(Phar::loadPhar($file2, 'alias.phar'));
    var_dump(memory_get_usage() - $before < (1 << 20));
    var_dump(file_get_contents('phar://alias.phar/a.txt'));
    var_dump(strlen(file_get_contents('phar://alias.phar/big.txt')));
} finally {
    @unlink("$dir/ondemand.phar");
    @unlink("$dir/alias.phar");
    @rmdir($dir);
}
