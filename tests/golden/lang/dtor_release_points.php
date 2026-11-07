<?php
// Where php releases a value, and so where its destructor runs (and throws).
class D {
    public function __construct(public string $n, public bool $throw = false) {}
    public function __destruct() {
        echo "~{$this->n}\n";
        if ($this->throw) {
            throw new Exception("from {$this->n}");
        }
    }
    public function __toString(): string { return $this->n; }
}

function mk(string $n, bool $throw = false): D { return new D($n, $throw); }

// A consumed temporary goes with the instruction that consumes it.
echo "concat: ", mk("a") . "!", "\n";
$x = (string) mk("b");
echo "cast done\n";
echo "prop: ", mk("c")->n, "\n";
var_dump(isset(mk("d")->n));
$arr = [1] + [mk("e")];
echo "add done\n";

// Its exception is caught by the try around it; the result is dropped.
try {
    var_dump(mk("f", true) . "x");
} catch (Exception $e) {
    echo "caught ", $e->getMessage(), " line ", $e->getLine(), "\n";
}

// static, global and =& drop the old value at their own line.
function st() {
    $s = new D("old-static");
    static $s;
    echo "after static\n";
}
st();
$g = new D("old-global");
function gl() {
    global $g;
    $g = null;
    echo "after global\n";
}
gl();
$r = new D("old-ref");
$t = 1;
$r = &$t;
echo "after ref\n";

// The catch variable's old value goes at the catch, outside the try.
function c() {
    $e = new D("old-e", true);
    try {
        try {
            throw new Exception("inner");
        } catch (Exception $e) {
            echo "unreached\n";
        }
    } catch (Exception $e2) {
        echo "outer caught ", $e2->getMessage(), " line ", $e2->getLine(), "\n";
    }
}
c();

// break through a finally frees the loops outside it after the finally ran.
function brk() {
    foreach ([0] as $_) {
        try {
            foreach (new D("loop", true) as $v) {
                try {
                    break 2;
                } finally {
                    echo "finally1\n";
                }
            }
        } catch (Exception $e) {
            echo "caught ", $e->getMessage(), "\n";
        } finally {
            echo "finally2\n";
        }
    }
}
brk();

// An exception leaving a frame frees its temporaries first.
function esc() {
    return [mk("tmp")] + throw new Exception("esc");
}
try {
    esc();
} catch (Exception $e) {
    echo "caught ", $e->getMessage(), "\n";
}

// array_splice destroys every removed element, chaining the exceptions.
$a = [new D("s1", true), new D("s2", true), 3];
try {
    array_splice($a, 0, 2);
} catch (Exception $e) {
    echo "caught ", $e->getMessage(), " prev ", $e->getPrevious()->getMessage(), "\n";
    echo count($a), " left\n";
}
echo "end\n";
