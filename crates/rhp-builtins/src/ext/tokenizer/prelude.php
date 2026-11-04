
class PhpToken implements Stringable {
    public int $id;
    public string $text;
    public int $line;
    public int $pos;

    final public function __construct(int $id, string $text, int $line = -1, int $pos = -1) {
        $this->id = $id;
        $this->text = $text;
        $this->line = $line;
        $this->pos = $pos;
    }
    public static function tokenize(string $code, int $flags = 0): array {
        $out = [];
        foreach (__token_list($code, $flags) as $t) {
            $out[] = new static($t[0], $t[1], $t[2], $t[3]);
        }
        return $out;
    }
    public function getTokenName(): ?string {
        if ($this->id < 256) { return chr($this->id); }
        $n = token_name($this->id);
        return $n === 'UNKNOWN' ? null : $n;
    }
    public function is($kind): bool {
        if (is_int($kind)) { return $this->id === $kind; }
        if (is_string($kind)) { return $this->text === $kind; }
        if (is_array($kind)) {
            foreach ($kind as $k) {
                if (is_int($k)) { if ($this->id === $k) { return true; } }
                elseif (is_string($k)) { if ($this->text === $k) { return true; } }
                else { throw new TypeError('PhpToken::is(): Argument #1 ($kind) must only have elements of type string|int, ' . get_debug_type($k) . ' given'); }
            }
            return false;
        }
        throw new TypeError('PhpToken::is(): Argument #1 ($kind) must be of type string|int|array, ' . get_debug_type($kind) . ' given');
    }
    public function isIgnorable(): bool {
        return $this->id === T_WHITESPACE || $this->id === T_COMMENT || $this->id === T_DOC_COMMENT || $this->id === T_OPEN_TAG;
    }
    public function __toString(): string { return $this->text; }
}
