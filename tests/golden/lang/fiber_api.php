<?php
$f = new Fiber(fn() => 1);
var_dump($f);
try { new Fiber(42); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
try { serialize($f); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
try { $f->x = 1; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { clone $f; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $f->getReturn(); } catch (FiberError $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { $f->resume(); } catch (FiberError $e) { echo $e->getMessage(), "\n"; }
try { $f->throw(1); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
var_dump(Fiber::getCurrent());
$f2 = new Fiber(function (int $a, ...$rest) { var_dump($a, $rest); return Fiber::suspend(); });
$f2->start(1, 2, x: 3);
try { $f2->start(); } catch (FiberError $e) { echo $e->getMessage(), "\n"; }
$f2->resume(9);
var_dump($f2->getReturn(), $f2->isTerminated());
$f3 = new Fiber('strlen');
try { $f3->start(); } catch (Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
$f4 = new Fiber(function () { throw new LogicException("boom"); });
try { $f4->start(); } catch (LogicException $e) { echo $e->getMessage(), "\n", str_replace(__FILE__, "F", $e->getTraceAsString()), "\n"; }
try { $f4->getReturn(); } catch (FiberError $e) { echo $e->getMessage(), "\n"; }
echo (new ReflectionClass('Fiber'))->isFinal() ? "final\n" : "no\n";
print_r(get_class_methods('Fiber'));
$g = function () { $x = Fiber::suspend(1); echo "in: $x\n"; Fiber::suspend(2); };
$a = new Fiber($g); $b = new Fiber($g);
echo $a->start(), $b->start(), "\n"; $b->resume('B'); $a->resume('A');
// A method callable keeps the scope and $this it was checked in.
class A {
    private function run($x) { echo "run $x\n"; Fiber::suspend(); throw new Exception("e"); }
    private static function srun() { echo "srun\n"; }
    function go() { $f = new Fiber([$this, 'run']); $f->start(5); (new Fiber([self::class, 'srun']))->start(); return $f; }
}
$f = (new A)->go();
try { $f->resume(); } catch (Exception $e) { echo str_replace(__FILE__, "F", $e->getTraceAsString()), "\n"; }
// Nested fibers, and error_reporting / @ kept per fiber.
set_error_handler(function ($n, $s) { if (error_reporting() & $n) { echo "error: $s\n"; } return true; });
error_reporting(E_ALL & ~E_USER_NOTICE);
$outer = new Fiber(function () {
    $inner = new Fiber(function () {
        echo "inner sees outer running: ", var_export(Fiber::getCurrent() !== null, true), "\n";
        error_reporting(E_ALL);
        Fiber::suspend('i1');
        trigger_error("inner notice", E_USER_NOTICE);
        return 'inner done';
    });
    echo $inner->start(), "\n";
    trigger_error("outer notice (hidden)", E_USER_NOTICE);
    Fiber::suspend($inner);
    @trigger_error("silenced", E_USER_WARNING);
});
$inner = $outer->start();
trigger_error("main notice (hidden)", E_USER_NOTICE);
$inner->resume();
echo $inner->getReturn(), "\n";
@$outer->resume();
var_dump($outer->isTerminated());
