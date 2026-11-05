<?php
// Property hooks: compound writes, isset/empty/??, indirect writes,
// object views, foreach order and parent hooks.

class Counter {
    public array $log = [];
    public int $n = 0 {
        get { $this->log[] = "get"; return $this->n; }
        set { $this->log[] = "set $value"; $this->n = $value; }
    }
    public string $s = "a" {
        set => strtoupper($value);
    }
    public $virt {
        get => $this->n * 10;
    }
    public ?string $maybe = null {
        get { echo "maybe::get\n"; return $this->maybe; }
    }
    private $secret = 's' {
        get => "<{$this->secret}>";
    }
}

$c = new Counter;
$c->n += 2;
$c->n++;
++$c->n;
$c->s .= "b";
var_dump($c->n, $c->s, $c->virt);
echo implode(",", $c->log), "\n";
var_dump(isset($c->maybe), empty($c->maybe), $c->maybe ?? "dflt");
var_dump(isset($c->virt), empty($c->virt));
$c->maybe = "x";
var_dump(isset($c->maybe));

try {
    $c->virt[] = 1;
} catch (Error $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
try {
    $c->virt = 1;
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
try {
    unset($c->n);
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}

$c->log = [];
var_dump(get_object_vars($c));
echo json_encode($c), "\n";
var_export($c);
echo "\n";
var_dump((array) $c);
foreach ($c as $k => $v) {
    echo "$k => ", json_encode($v), "\n";
}

// parent::$p::get() / ::set() with and without a parent hook.
class P {
    public $plain = 1;
    public $hooked = 1 {
        set { echo "P::set\n"; $this->hooked = $value * 2; }
    }
}
class Q extends P {
    public $plain {
        get => parent::$plain::get() + 100;
        set { parent::$plain::set($value + 1); }
    }
    public $hooked {
        set { echo "Q::set\n"; parent::$hooked::set($value + 1); }
    }
}
$q = new Q;
$q->plain = 5;
$q->hooked = 5;
var_dump($q->plain, $q->hooked);
print_r($q);

// Hooks from a trait, __PROPERTY__, and a closure made in a hook.
trait Named {
    public string $name = "" {
        set => trim($value);
        get { return __PROPERTY__ . "=" . $this->name; }
    }
}
class User {
    use Named;
    public $f { get => fn() => $this->name; }
}
$u = new User;
$u->name = "  bob ";
var_dump($u->name, ($u->f)());

// Interface properties.
interface HasId {
    public int $id { get; }
}
class Item implements HasId {
    public function __construct(public int $id) {}
}
$i = new Item(7);
var_dump($i instanceof HasId, $i->id);
$r = new ReflectionProperty(HasId::class, 'id');
var_dump($r->isAbstract(), $r->isVirtual(), $r->hasHooks(), $r->hasHook(PropertyHookType::Set));

// Promoted property with a hook.
class Temp {
    public function __construct(
        public float $celsius {
            set { echo "set $value\n"; $this->celsius = $value; }
        },
    ) {}
    public float $fahrenheit {
        get => $this->celsius * 9 / 5 + 32;
        set => $this->celsius = ($value - 32) * 5 / 9;
    }
}
$t = new Temp(100);
var_dump($t->fahrenheit);
$t->fahrenheit = 32;
var_dump($t->celsius);
print_r($t);

// ErrorException keeps its previous exception.
$e = new ErrorException("m", 1, E_WARNING, "f", 1, new Exception("prev"));
var_dump($e->getPrevious()->getMessage());
