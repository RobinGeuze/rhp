<?php
// A source scanner in the style of Composer's class-map generator: skip
// strings and comments with strcspn() from an offset, peek at characters
// through $this, collect class names.
class Scanner {
    private string $src;
    private int $len;
    private int $index = 0;
    private array $found = [];

    function __construct(string $src) {
        $this->src = $src;
        $this->len = strlen($src);
    }

    private function peek(string $c): bool {
        return $this->index + 1 < $this->len && $this->src[$this->index + 1] === $c;
    }

    private function skipTo(string $mask): void {
        $this->index += strcspn($this->src, $mask, $this->index) + 1;
    }

    function scan(): array {
        while ($this->index < $this->len) {
            $c = $this->src[$this->index];
            if ($c === '"' || $c === "'") {
                $this->index++;
                $this->skipTo($c);
            } elseif ($c === '/' && $this->peek('/')) {
                $this->skipTo("\n");
            } elseif ($c === 'c' && substr($this->src, $this->index, 6) === 'class ') {
                $this->index += 6;
                $n = strcspn($this->src, " {\n", $this->index);
                $this->found[] = substr($this->src, $this->index, $n);
                $this->index += $n;
            } else {
                $this->index++;
            }
        }
        return $this->found;
    }
}

$unit = "<?php\n// a comment with class words\nclass Foo%d extends Bar {\n"
    . "    public \$s = 'a string with class inside';\n"
    . "    function f() { return \"x\" . 'y'; }\n}\n";
$src = '';
for ($i = 0; $i < 400; $i++) {
    $src .= sprintf($unit, $i);
}
$t = 0;
for ($i = 0; $i < 20; $i++) {
    $t += count((new Scanner($src))->scan());
}
echo $t, "\n";
