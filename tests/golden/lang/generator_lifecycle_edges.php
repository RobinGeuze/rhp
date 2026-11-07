<?php
// Generator lifecycle corners: keys around `yield from`, destruction running
// finally blocks, throw() into delegates, return values and exception chains
// through finally.

function keys() {
    yield 'a';
    yield from ['x' => 1, 5 => 2];
    yield 'b';
    yield 10 => 'c';
    yield 'd';
}
foreach (keys() as $k => $v) {
    echo var_export($k, true), " => ", var_export($v, true), "\n";
}

function inner() {
    $x = yield 1;
    echo "inner got ", var_export($x, true), "\n";
    yield 2;
    return 'inner-ret';
}
function outer() {
    $r = yield from inner();
    echo "outer got ", $r, "\n";
    return 'outer-ret';
}
$g = outer();
var_dump($g->current());
var_dump($g->send('sent'));
$g->next();
var_dump($g->valid(), $g->getReturn());

// A generator that already returned cannot be yielded from.
$done = inner();
foreach ($done as $_) {}
function from_done($g) { yield from $g; }
try {
    foreach (from_done($done) as $_) {}
} catch (Error $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}

// Destroying runs the pending finally blocks, inner first, then outer.
function nested() {
    try {
        try {
            yield 1;
        } finally {
            echo "inner finally\n";
        }
        echo "not reached\n";
    } finally {
        echo "outer finally\n";
    }
}
$n = nested();
$n->current();
unset($n);
echo "after unset\n";

// An exception from a finally run on destruction reaches the releasing code.
function throws_on_close() {
    try {
        yield;
    } finally {
        throw new RuntimeException("from finally");
    }
}
try {
    $t = throws_on_close();
    $t->current();
    $t = null;
} catch (RuntimeException $e) {
    echo "caught: ", $e->getMessage(), "\n";
}

// A return in that finally ends it normally.
function returns_on_close() {
    try {
        yield;
    } finally {
        echo "returning\n";
        return 5;
    }
}
$r = returns_on_close();
$r->current();
unset($r);

// throw() goes to the innermost delegate; the outer one may catch it.
function leaf() {
    try {
        yield 'leaf';
    } finally {
        echo "leaf finally\n";
    }
}
function mid() {
    try {
        yield from leaf();
    } catch (LogicException $e) {
        echo "mid caught ", $e->getMessage(), "\n";
        yield 'mid';
    }
}
$m = mid();
var_dump($m->current());
var_dump($m->throw(new LogicException('boom')));

// After the body threw, the generator is closed.
function dies() {
    yield 1;
    throw new Exception('dead');
}
$d = dies();
$d->current();
try {
    $d->next();
} catch (Exception $e) {
    echo $e->getMessage(), "\n";
}
var_dump($d->current(), $d->key(), $d->valid());
try {
    $d->getReturn();
} catch (Exception $e) {
    echo $e->getMessage(), "\n";
}

// A pending exception becomes the previous one of an exception thrown in
// its finally block.
function chain() {
    try {
        throw new Exception('first');
    } finally {
        foreach ([1] as $_) {
            try {
                break;
            } finally {
                echo "loop finally\n";
            }
        }
        throw new Exception('second');
    }
}
try {
    chain();
} catch (Exception $e) {
    echo $e->getMessage(), " <- ", $e->getPrevious()?->getMessage(), "\n";
}
