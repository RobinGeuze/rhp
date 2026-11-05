<?php
class Model {
    protected static int $instances = 0;
    public static function create(): static {
        static::$instances++;
        return new static();
    }
    public static function count(): int { return static::$instances; }
    public function type(): string { return static::class; }
}
class User extends Model {
    protected static int $instances = 0;
}
class Post extends Model {
    protected static int $instances = 0;
}
$u1 = User::create();
$u2 = User::create();
$p1 = Post::create();
echo "users: " . User::count() . "\n";
echo "posts: " . Post::count() . "\n";
echo $u1->type(), " ", $p1->type(), "\n";

function counter(): int {
    static $n = 0;
    return ++$n;
}
echo counter(), counter(), counter(), "\n";

class Registry {
    private static array $items = [];
    public static function set(string $k, $v): void { self::$items[$k] = $v; }
    public static function get(string $k) { return self::$items[$k] ?? null; }
    public static function all(): array { return self::$items; }
}
Registry::set('a', 1);
Registry::set('b', 2);
print_r(Registry::all());
