//! Unit suite part 2: functions, closures, OOP, magic methods, exceptions,
//! enums, named args, global/static vars. Exact-output assertions.

mod common;
use common::{run, run_throws};

// ---------------------------------------------------------------------------
// Functions: defaults, variadics, recursion, hoisting, named args
// ---------------------------------------------------------------------------

#[test]
fn default_parameters() {
    assert_eq!(
        run("<?php function f($a, $b = 10) { return $a + $b; } echo f(5), ' ', f(5, 20);"),
        "15 25"
    );
}

#[test]
fn variadic_collects_rest() {
    assert_eq!(run("<?php function f($a, $b, ...$rest){return \"$a-$b-\".implode(',', $rest);} echo f(1,2,3,4,5);"), "1-2-3,4,5");
    assert_eq!(run("<?php function f($a, $b, ...$rest){return \"$a-$b-\".implode(',', $rest);} echo f(1,2);"), "1-2-");
}

#[test]
fn recursion_fibonacci() {
    assert_eq!(
        run("<?php function fib($n){return $n<2 ? $n : fib($n-1)+fib($n-2);} echo fib(10);"),
        "55"
    );
}

#[test]
fn function_hoisting_call_before_definition() {
    assert_eq!(
        run("<?php echo later(); function later(){return 'ok';}"),
        "ok"
    );
}

#[test]
fn named_arguments() {
    let greet = "<?php function greet(string $name, string $greeting='Hello', string $punct='!'){return \"$greeting, $name$punct\";}";
    assert_eq!(
        run(&format!("{greet} echo greet(name: 'Bob');")),
        "Hello, Bob!"
    );
    assert_eq!(
        run(&format!("{greet} echo greet(name: 'Al', greeting: 'Hi');")),
        "Hi, Al!"
    );
    assert_eq!(
        run(&format!("{greet} echo greet('Cy', punct: '?');")),
        "Hello, Cy?"
    );
    assert_eq!(
        run(&format!(
            "{greet} echo greet(greeting: 'Hey', name: 'Dee');"
        )),
        "Hey, Dee!"
    );
}

#[test]
fn named_arguments_skip_middle_default() {
    assert_eq!(run("<?php function box(int $w, int $h=10, int $d=1){return $w*$h*$d;} echo box(5), ' ', box(5, d: 3), ' ', box(w: 2, h: 3, d: 4);"), "50 150 24");
}

// ---------------------------------------------------------------------------
// Closures and first-class callables
// ---------------------------------------------------------------------------

#[test]
fn closure_captures_by_value_at_definition() {
    assert_eq!(
        run("<?php $x=10; $f=function() use ($x){return $x;}; $x=20; echo $f();"),
        "10"
    );
}

#[test]
fn arrow_function_captures_automatically() {
    assert_eq!(run("<?php $m=3; $f=fn($x)=>$x*$m; echo $f(4);"), "12");
}

#[test]
fn nested_closures_capture_outer() {
    assert_eq!(
        run("<?php function make($n){return function($x) use ($n){return function($y) use ($n,$x){return $n+$x+$y;};};} $f=make(1); $g=$f(2); echo $g(3);"),
        "6"
    );
}

#[test]
fn first_class_callable_from_user_function() {
    assert_eq!(
        run("<?php function sq($x){return $x*$x;} $f=sq(...); echo $f(5);"),
        "25"
    );
}

#[test]
fn first_class_callable_from_builtin() {
    assert_eq!(run("<?php $g=strlen(...); echo $g('abc');"), "3");
}

#[test]
fn closure_as_callback_in_registry() {
    assert_eq!(
        run("<?php $fns=[]; $fns['double']=fn($x)=>$x*2; $fns['greet']=fn($n)=>\"Hi $n\"; echo ($fns['double'])(21), '|', ($fns['greet'])('Al');"),
        "42|Hi Al"
    );
}

// ---------------------------------------------------------------------------
// OOP: methods, inheritance, visibility, constructor promotion
// ---------------------------------------------------------------------------

#[test]
fn basic_method_call_and_property() {
    assert_eq!(run("<?php class C{public int $v=0; public function set($n){$this->v=$n;} public function get(){return $this->v;}} $c=new C(); $c->set(42); echo $c->get();"), "42");
}

#[test]
fn inheritance_virtual_dispatch() {
    assert_eq!(
        run("<?php class A{public function hi(){return 'A';} public function who(){return $this->hi();}} class B extends A{public function hi(){return 'B';}} echo (new B())->who();"),
        "B"
    );
}

#[test]
fn parent_call() {
    assert_eq!(
        run("<?php class A{public function x(){return 1;}} class B extends A{public function x(){return parent::x()+1;}} echo (new B())->x();"),
        "2"
    );
}

#[test]
fn private_property_access_blocked_outside() {
    let (_out, class) = run_throws("<?php class A{private $s='secret';} $a=new A(); echo $a->s;");
    assert_eq!(class, "Error");
}

#[test]
fn protected_property_message() {
    // From golden magic_methods: accessing protected prop externally throws with this exact message.
    assert_eq!(
        run("<?php class A{protected float $b=0.0;} $a=new A(); try{echo $a->b;}catch(\\Error $e){echo $e->getMessage();}"),
        "Cannot access protected property A::$b"
    );
}

#[test]
fn constructor_property_promotion() {
    assert_eq!(run("<?php class P{public function __construct(public int $x, public int $y=5){}} $p=new P(3); echo $p->x, ',', $p->y;"), "3,5");
    assert_eq!(run("<?php class P{public function __construct(public int $x, public int $y=5){}} $q=new P(1,2); echo $q->x, ',', $q->y;"), "1,2");
}

#[test]
fn method_chaining_returns_this() {
    assert_eq!(
        run("<?php class Q{private $p=[]; public function add($x){$this->p[]=$x; return $this;} public function build(){return implode(',', $this->p);}} echo (new Q())->add(1)->add(2)->add(3)->build();"),
        "1,2,3"
    );
}

#[test]
fn clone_is_shallow_copy_independent() {
    assert_eq!(
        run(
            "<?php class P{public $n=1;} $a=new P(); $b=clone $a; $b->n=5; echo $a->n, ' ', $b->n;"
        ),
        "1 5"
    );
}

#[test]
fn instanceof_class_and_interface() {
    assert_eq!(
        run("<?php interface I{} class C implements I{} $c=new C(); var_dump($c instanceof I);"),
        "bool(true)\n"
    );
    assert_eq!(
        run("<?php interface I{} class C implements I{} $c=new C(); var_dump($c instanceof C);"),
        "bool(true)\n"
    );
    assert_eq!(
        run("<?php class A{} class B{} var_dump(new A() instanceof B);"),
        "bool(false)\n"
    );
}

#[test]
fn traits_mixed_in() {
    assert_eq!(run("<?php trait T{public function hi(){return 'hi from '.get_class($this);}} class C{use T;} echo (new C())->hi();"), "hi from C");
}

// ---------------------------------------------------------------------------
// Static properties, constants, late static binding
// ---------------------------------------------------------------------------

#[test]
fn static_property_shared_across_instances() {
    assert_eq!(run("<?php class C{public static int $c=0; public function __construct(){self::$c++;}} new C(); new C(); echo C::$c;"), "2");
}

#[test]
fn class_constants_and_self_reference() {
    assert_eq!(
        run("<?php class C{const X=10; const Y=20;} echo C::X, ' ', C::Y;"),
        "10 20"
    );
}

#[test]
fn interface_constant_access() {
    assert_eq!(run("<?php interface I{const V=42;} echo I::V;"), "42");
}

#[test]
fn late_static_binding_new_static() {
    assert_eq!(
        run("<?php class A{public static function create(){return new static();} public function name(){return static::class;}} class B extends A{} echo (B::create())->name();"),
        "B"
    );
}

#[test]
fn late_static_binding_const_dispatch() {
    // From golden oop_static: static::KIND resolves on the runtime class.
    assert_eq!(
        run("<?php class Base{const KIND='base'; public function kind(){return static::KIND;}} class Derived extends Base{const KIND='derived';} $a=new Base(); $b=new Derived(); echo $a->kind(), ' ', $b->kind();"),
        "base derived"
    );
}

// ---------------------------------------------------------------------------
// Magic methods
// ---------------------------------------------------------------------------

#[test]
fn magic_get_set_isset_unset() {
    let cls = "<?php class C{private array $d=[]; public function __get($n){return $this->d[$n] ?? null;} public function __set($n,$v){$this->d[$n]=$v;} public function __isset($n){return isset($this->d[$n]);} public function __unset($n){unset($this->d[$n]);}}";
    assert_eq!(
        run(&format!(
            "{cls} $c=new C(); $c->x=10; $c->y=20; echo $c->x + $c->y;"
        )),
        "30"
    );
    assert_eq!(
        run(&format!(
            "{cls} $c=new C(); $c->x=1; var_dump(isset($c->x));"
        )),
        "bool(true)\n"
    );
    assert_eq!(
        run(&format!("{cls} $c=new C(); var_dump(isset($c->missing));")),
        "bool(false)\n"
    );
    assert_eq!(
        run(&format!(
            "{cls} $c=new C(); $c->x=1; unset($c->x); var_dump(isset($c->x));"
        )),
        "bool(false)\n"
    );
    assert_eq!(
        run(&format!("{cls} $c=new C(); echo $c->gone ?? 'none';")),
        "none"
    );
}

#[test]
fn magic_call_and_callstatic() {
    assert_eq!(run("<?php class C{public function __call($n,$a){return \"$n(\".implode(',', $a).')';}} $c=new C(); echo $c->compute(1,2,3);"), "compute(1,2,3)");
    assert_eq!(run("<?php class C{public static function __callStatic($n,$a){return \"static $n\";}} echo C::make();"), "static make");
}

#[test]
fn magic_invoke() {
    assert_eq!(run("<?php class C{public function __invoke(...$a){return array_sum($a);}} $c=new C(); echo $c(4,5,6);"), "15");
}

#[test]
fn magic_tostring_in_contexts() {
    let cls = "<?php class M{public function __construct(public int $v){} public function __toString(){return 'M('.$this->v.')';}}";
    assert_eq!(run(&format!("{cls} $m=new M(7); echo $m;")), "M(7)");
    assert_eq!(
        run(&format!("{cls} $m=new M(7); echo \"val=$m\";")),
        "val=M(7)"
    );
    assert_eq!(run(&format!("{cls} $m=new M(7); echo (string)$m;")), "M(7)");
}

#[test]
fn nullsafe_operator() {
    let cls = "<?php class C{public ?C $next=null; public int $v=0;}";
    assert_eq!(
        run(&format!(
            "{cls} $a=new C(); $a->v=1; echo $a->next?->v ?? 'none';"
        )),
        "none"
    );
    assert_eq!(
        run(&format!(
            "{cls} $a=new C(); $a->next=new C(); $a->next->v=9; echo $a->next?->v;"
        )),
        "9"
    );
}

// ---------------------------------------------------------------------------
// Closures capturing $this
// ---------------------------------------------------------------------------

#[test]
fn closure_captures_this_arrow() {
    assert_eq!(
        run("<?php class Counter{private int $n; public function __construct(int $s){$this->n=$s;} public function adder():callable{return fn($x)=>$this->n+$x;}} $c=new Counter(10); $a=$c->adder(); echo $a(5);"),
        "15"
    );
}

#[test]
fn closure_mutates_this_over_calls() {
    assert_eq!(
        run("<?php class Counter{private int $n=10; public function inc():callable{return function(){$this->n++; return $this->n;};} public function value(){return $this->n;}} $c=new Counter(); $f=$c->inc(); echo $f(), ' ', $f(), ' ', $f(), ' ', $c->value();"),
        "11 12 13 13"
    );
}

// ---------------------------------------------------------------------------
// Exceptions
// ---------------------------------------------------------------------------

#[test]
fn try_catch_basic() {
    assert_eq!(
        run("<?php try{throw new Exception('boom');}catch(Exception $e){echo $e->getMessage();}"),
        "boom"
    );
}

#[test]
fn catch_most_specific_type() {
    let f = "<?php function f($x){try{if($x==1)throw new InvalidArgumentException('bad'); if($x==2)throw new RuntimeException('run'); return 'ok';}catch(InvalidArgumentException $e){return 'iae:'.$e->getMessage();}catch(RuntimeException $e){return 'rte:'.$e->getMessage();}}";
    assert_eq!(
        run(&format!("{f} echo f(0), '|', f(1), '|', f(2);")),
        "ok|iae:bad|rte:run"
    );
}

#[test]
fn finally_runs_after_try_return() {
    assert_eq!(
        run("<?php function f(){try{return 'try';}finally{echo 'fin ';}} echo f();"),
        "fin try"
    );
}

#[test]
fn finally_runs_after_catch_return() {
    assert_eq!(
        run("<?php function g(){try{throw new Exception('x');}catch(Exception $e){return 'caught';}finally{echo 'gfin ';}} echo g();"),
        "gfin caught"
    );
}

#[test]
fn finally_return_overrides_try_return() {
    assert_eq!(
        run("<?php function f(){try{return 'a';}finally{return 'b';}} echo f();"),
        "b"
    );
}

#[test]
fn exception_unwinds_across_frames() {
    assert_eq!(
        run("<?php function deep(){throw new RuntimeException('boom');} function mid(){deep();} try{mid();}catch(Exception $e){echo get_class($e), ': ', $e->getMessage();}"),
        "RuntimeException: boom"
    );
}

#[test]
fn uncaught_exception_propagates_as_throw() {
    let (_out, class) = run_throws("<?php throw new Exception('nope');");
    assert_eq!(class, "Object");
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[test]
fn pure_enum_cases_and_identity() {
    assert_eq!(
        run("<?php enum Suit{case Hearts; case Spades;} echo Suit::Hearts->name;"),
        "Hearts"
    );
    assert_eq!(
        run("<?php enum Suit{case Hearts; case Spades;} var_dump(Suit::Hearts === Suit::Hearts);"),
        "bool(true)\n"
    );
    assert_eq!(
        run("<?php enum Suit{case Hearts; case Spades;} var_dump(Suit::Hearts === Suit::Spades);"),
        "bool(false)\n"
    );
    assert_eq!(
        run("<?php enum Suit{case Hearts; case Spades;} echo count(Suit::cases());"),
        "2"
    );
}

#[test]
fn enum_method_with_match_on_self() {
    assert_eq!(
        run("<?php enum Dir{case N; case S; public function opp():Dir{return match($this){Dir::N=>Dir::S,Dir::S=>Dir::N};}} echo Dir::N->opp()->name;"),
        "S"
    );
}

#[test]
fn backed_enum_value_from_tryfrom() {
    assert_eq!(
        run("<?php enum Code:int{case Ok=200; case NotFound=404;} echo Code::Ok->value;"),
        "200"
    );
    assert_eq!(
        run("<?php enum Code:int{case Ok=200; case NotFound=404;} echo Code::from(404)->name;"),
        "NotFound"
    );
    assert_eq!(
        run("<?php enum Code:int{case Ok=200; case NotFound=404;} var_dump(Code::tryFrom(999));"),
        "NULL\n"
    );
}

#[test]
fn backed_enum_from_invalid_throws_value_error() {
    let (_out, class) = run_throws("<?php enum Size:int{case S=1;} Size::from(99);");
    assert_eq!(class, "ValueError");
}

#[test]
fn enum_var_dump_shape_and_interfaces() {
    assert_eq!(
        run("<?php enum S:string{case A='a';} var_dump(S::A);"),
        "enum(S::A)\n"
    );
    assert_eq!(
        run("<?php enum S:string{case A='a';} var_dump(S::A instanceof UnitEnum);"),
        "bool(true)\n"
    );
    assert_eq!(
        run("<?php enum S:string{case A='a';} var_dump(S::A instanceof BackedEnum);"),
        "bool(true)\n"
    );
    assert_eq!(
        run("<?php enum S{case A;} var_dump(S::A instanceof BackedEnum);"),
        "bool(false)\n"
    );
}

// ---------------------------------------------------------------------------
// Global and static variables
// ---------------------------------------------------------------------------

#[test]
fn global_keyword_binds_to_main_scope() {
    assert_eq!(run("<?php $counter=0; function bump(){global $counter; $counter++;} bump(); bump(); bump(); echo $counter;"), "3");
}

#[test]
fn global_can_create_new_binding() {
    assert_eq!(
        run("<?php function init(){global $fresh; $fresh='hi';} init(); echo $fresh;"),
        "hi"
    );
}

#[test]
fn global_array_append() {
    assert_eq!(
        run("<?php $log=[]; function record($m){global $log; $log[]=$m;} record('first'); record('second'); echo implode(',', $log);"),
        "first,second"
    );
}

#[test]
fn static_variable_persists_across_calls() {
    assert_eq!(
        run("<?php function tick(){static $n=0; return ++$n;} echo tick(), tick(), tick();"),
        "123"
    );
}

#[test]
fn static_initializer_is_const_expr() {
    assert_eq!(
        run("<?php function acc($x){static $t=2*5; $t+=$x; return $t;} echo acc(5), ' ', acc(3);"),
        "15 18"
    );
}

#[test]
fn global_swap_via_destructuring() {
    assert_eq!(run("<?php $a=1; $b=2; function swap(){global $a,$b; [$a,$b]=[$b,$a];} swap(); echo \"$a $b\";"), "2 1");
}
