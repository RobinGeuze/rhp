<?php
// A small revolt-style event loop: a queue of fibers, deferred callbacks,
// timers on a fake tick clock, suspension from nested calls, from inside
// array_map and usort callbacks, from a destructor and from a generator.

final class Loop {
    private static array $queue = [];
    private static array $deferred = [];
    private static array $timers = [];
    private static int $now = 0;

    public static function defer(Closure $cb): void { self::$deferred[] = $cb; }

    public static function delay(int $ticks, Closure $cb): void {
        self::$timers[] = [self::$now + $ticks, count(self::$timers), $cb];
    }

    public static function spawn(Closure $fn, mixed ...$args): Fiber {
        $f = new Fiber($fn);
        self::$queue[] = [$f, $args, true];
        return $f;
    }

    /** Suspend the running fiber until the loop resumes it with a value. */
    public static function await(int $ticks): mixed {
        $f = Fiber::getCurrent() ?? throw new LogicException('not in a fiber');
        $at = self::$now;
        self::delay($ticks, fn() => self::$queue[] = [$f, ["woke@$at+$ticks"], false]);
        return Fiber::suspend('waiting');
    }

    public static function run(): void {
        while (self::$queue || self::$deferred || self::$timers) {
            while ($cb = array_shift(self::$deferred)) {
                $cb();
            }
            while (self::$queue) {
                [$f, $args, $start] = array_shift(self::$queue);
                $r = $start ? $f->start(...$args) : $f->resume($args[0]);
                if ($f->isTerminated()) {
                    echo "  fiber done: ", var_export($f->getReturn(), true), "\n";
                } else {
                    echo "  fiber says: $r\n";
                }
            }
            if (self::$timers) {
                usort(self::$timers, fn($a, $b) => [$a[0], $a[1]] <=> [$b[0], $b[1]]);
                [$at, , $cb] = array_shift(self::$timers);
                self::$now = max(self::$now, $at);
                echo "tick ", self::$now, "\n";
                $cb();
            }
        }
    }
}

function nested(int $depth, string $name): string {
    if ($depth === 0) {
        return Loop::await(2);
    }
    return nested($depth - 1, $name) . "<$depth";
}

class Guard {
    public function __construct(private string $name) {}
    public function __destruct() {
        echo "  {$this->name}: destructor suspends\n";
        echo "  {$this->name}: destructor got ", Loop::await(1), "\n";
    }
}

function numbers() {
    for ($i = 1; $i <= 2; $i++) {
        $got = Loop::await($i);
        yield $i => $got;
    }
}

Loop::spawn(function (string $name) {
    echo "$name: start\n";
    echo "$name: ", nested(3, $name), "\n";
    return "$name finished";
}, 'A');

Loop::spawn(function () {
    $out = array_map(fn($n) => $n . ':' . Loop::await($n), [3, 1]);
    echo "B: ", implode(', ', $out), "\n";
    $list = [3, 1, 2];
    usort($list, function ($a, $b) {
        Loop::await(1);
        return $a <=> $b;
    });
    echo "B: sorted ", implode(',', $list), "\n";
    return count($out);
});

Loop::spawn(function () {
    $g = new Guard('C');
    unset($g);
    foreach (numbers() as $k => $v) {
        echo "C: generator $k => $v\n";
    }
    try {
        Loop::await(1);
        throw new RuntimeException('C failed');
    } finally {
        echo "C: finally\n";
    }
});

Loop::defer(function () {
    echo "deferred runs first\n";
    Loop::defer(fn() => print("nested deferred\n"));
});

try {
    Loop::run();
} catch (RuntimeException $e) {
    echo "loop stopped: ", $e->getMessage(), "\n";
    echo str_replace(__FILE__, "F", $e->getTraceAsString()), "\n";
}

// An abandoned fiber is destroyed: its finally block runs, a catch does not.
$f = new Fiber(function () {
    try {
        Fiber::suspend('parked');
    } catch (Throwable $t) {
        echo "not reached\n";
    } finally {
        echo "abandoned fiber unwinds\n";
    }
});
echo $f->start(), "\n";
unset($f);
echo "end\n";
