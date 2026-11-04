
final class GMP {
    public function __construct($num = 0, $base = 0) { __gmp_construct($this, $num, $base); }
    public function __serialize() { return __gmp_serialize($this); }
    public function __unserialize($data) { __gmp_unserialize($this, $data); }
}
