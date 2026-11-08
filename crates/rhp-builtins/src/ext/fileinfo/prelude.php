class finfo {
    public function __construct(int $flags = FILEINFO_NONE, ?string $magic_database = null) { __finfo('__construct', $this, func_num_args(), $flags, $magic_database); }
    public function file(string $filename, int $flags = FILEINFO_NONE, $context = null): string|false { return __finfo('file', $this, func_num_args(), $filename, $flags, $context); }
    public function buffer(string $string, int $flags = FILEINFO_NONE, $context = null): string|false { return __finfo('buffer', $this, func_num_args(), $string, $flags, $context); }
    public function set_flags(int $flags): true { return __finfo('set_flags', $this, func_num_args(), $flags); }
}
