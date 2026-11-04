
final class HashContext {
    private $__algo = ''; private $__d = ''; private $__k = null; private $__done = false; private $__init = null;
    private function __construct() {}
    public function __debugInfo(): array { return ['algo' => $this->__algo]; }
    public static function __make(string $algo, $key, $init = null): HashContext {
        $c = new HashContext(); $c->__algo = $algo; $c->__k = $key; $c->__init = $init; return $c;
    }
    public static function __feed(HashContext $c, string $fn, string $data): void {
        if ($c->__done) { throw new TypeError("$fn(): Argument #1 (\$context) must be a valid, non-finalized HashContext"); }
        $c->__d .= $data;
    }
    public static function __final(HashContext $c, bool $binary): string {
        if ($c->__done) { throw new TypeError("hash_final(): Argument #1 (\$context) must be a valid, non-finalized HashContext"); }
        $c->__done = true;
        $r = $c->__k === null ? __hash_digest($c->__algo, $c->__d, $binary, $c->__init) : hash_hmac($c->__algo, $c->__d, $c->__k, $binary);
        $c->__d = '';
        return $r;
    }
    public static function __copy(HashContext $c): HashContext {
        if ($c->__done) { throw new TypeError("hash_copy(): Argument #1 (\$context) must be a valid, non-finalized HashContext"); }
        $n = HashContext::__make($c->__algo, $c->__k, $c->__init); $n->__d = $c->__d; return $n;
    }
}
function hash_init(string $algo, int $flags = 0, string $key = "", array $options = []): HashContext {
    $algo = strtolower($algo);
    if (!in_array($algo, hash_algos(), true)) {
        throw new ValueError('hash_init(): Argument #1 ($algo) must be a valid hashing algorithm');
    }
    if ($flags & HASH_HMAC) {
        if (!in_array($algo, hash_hmac_algos(), true)) {
            throw new ValueError('hash_init(): Argument #1 ($algo) must be a cryptographic hashing algorithm if HMAC is requested');
        }
        if ($key === '') {
            throw new ValueError('hash_init(): Argument #3 ($key) must not be empty when HMAC is requested');
        }
        return HashContext::__make($algo, $key);
    }
    return HashContext::__make($algo, null, func_num_args() > 3 ? __hash_init_options($algo, $options) : null);
}
function hash_update(HashContext $context, string $data): true {
    HashContext::__feed($context, 'hash_update', $data);
    return true;
}
function hash_update_stream(HashContext $context, $stream, int $length = -1): int {
    $data = $length < 0 ? stream_get_contents($stream) : fread($stream, $length);
    if ($data === false) { $data = ''; }
    HashContext::__feed($context, 'hash_update_stream', $data);
    return strlen($data);
}
function hash_update_file(HashContext $context, string $filename, $stream_context = null): bool {
    $data = @file_get_contents($filename);
    if ($data === false) {
        $e = error_get_last();
        if ($e !== null && str_starts_with($e['message'], 'file_get_contents(')) {
            __stdext_warning('hash_update_file(' . substr($e['message'], strlen('file_get_contents(')));
        }
        return false;
    }
    HashContext::__feed($context, 'hash_update_file', $data);
    return true;
}
function hash_final(HashContext $context, bool $binary = false): string {
    return HashContext::__final($context, $binary);
}
function hash_copy(HashContext $context): HashContext {
    return HashContext::__copy($context);
}
