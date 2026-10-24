<?php
class LRUCache {
    private array $cache = [];
    public function __construct(private int $capacity) {}
    public function get(string $key): mixed {
        if (!isset($this->cache[$key])) return null;
        $val = $this->cache[$key];
        unset($this->cache[$key]);
        $this->cache[$key] = $val;
        return $val;
    }
    public function put(string $key, mixed $val): void {
        if (isset($this->cache[$key])) unset($this->cache[$key]);
        $this->cache[$key] = $val;
        if (count($this->cache) > $this->capacity) {
            array_shift($this->cache);
        }
    }
    public function keys(): array { return array_keys($this->cache); }
}
$c = new LRUCache(3);
$c->put('a', 1);
$c->put('b', 2);
$c->put('c', 3);
$c->get('a');
$c->put('d', 4);
echo implode(",", $c->keys()), "\n";
var_dump($c->get('b'));
var_dump($c->get('a'));
