<?php
// A tokenizer-like loop: methods reading and writing $this's properties
// (inherited, protected and private), static calls, instanceof and class
// constants, as parsers and analysers do.
interface Node {}
abstract class Base implements Node {
    const KIND = 1;
    protected int $pos = 0;
    protected string $src;
    private array $seen = [];
    function __construct(string $src) { $this->src = $src; }
    protected function peek(): string { return $this->src[$this->pos] ?? ''; }
    function mark(string $c): void { $this->seen[$c] = ($this->seen[$c] ?? 0) + 1; }
    function seen(): int { return count($this->seen); }
}
final class Lexer extends Base {
    public int $words = 0;
    public ?Node $last = null;
    static function isWord(string $c): bool { return $c !== ' ' && $c !== "\n"; }
    function run(): int {
        $len = strlen($this->src);
        while ($this->pos < $len) {
            $c = $this->peek();
            if (self::isWord($c) && !Lexer::isWord($this->src[$this->pos - 1] ?? ' ')) {
                $this->words++;
                $this->mark($c);
            }
            if ($this instanceof Node && $this->last === null) {
                $this->last = $this;
            }
            $this->pos += static::KIND;
        }
        return $this->words + $this->seen();
    }
}
$src = str_repeat("lorem ipsum dolor sit amet\nconsectetur adipiscing elit ", 2000);
$t = 0;
for ($i = 0; $i < 4; $i++) {
    $l = new Lexer($src);
    $t += $l->run();
}
echo $t, "\n";
