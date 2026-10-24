<?php
class EventEmitter {
    private array $listeners = [];
    public function on(string $event, callable $cb): void {
        $this->listeners[$event][] = $cb;
    }
    public function emit(string $event, ...$args): array {
        $results = [];
        foreach ($this->listeners[$event] ?? [] as $cb) {
            $results[] = $cb(...$args);
        }
        return $results;
    }
}
$e = new EventEmitter();
$e->on('calc', fn($a, $b) => $a + $b);
$e->on('calc', fn($a, $b) => $a * $b);
print_r($e->emit('calc', 3, 4));

$log = [];
$e->on('log', function ($msg) use (&$log) { $log[] = strtoupper($msg); });
$e->emit('log', 'hello');
$e->emit('log', 'world');
echo implode(",", $log), "\n";
print_r($e->emit('nonexistent'));
