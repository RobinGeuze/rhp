<?php
class MyException extends RuntimeException {}
function level3() { throw new MyException("deep error"); }
function level2() { level3(); }
function level1() {
    try {
        level2();
    } catch (LogicException $e) {
        return "logic";
    }
    return "no catch here";
}
try {
    echo level1(), "\n";
} catch (RuntimeException $e) {
    echo "caught at top: ", $e->getMessage(), "\n";
    echo "is runtime: ", ($e instanceof RuntimeException) ? "yes" : "no", "\n";
    echo "is exception: ", ($e instanceof Exception) ? "yes" : "no", "\n";
    echo "class: ", get_class($e), "\n";
}
// finally runs even with return
function withFinally() {
    try {
        return "from try";
    } finally {
        echo "finally ran\n";
    }
}
echo withFinally(), "\n";
// rethrow
try {
    try {
        throw new Exception("original");
    } catch (Exception $e) {
        throw new RuntimeException("wrapped: " . $e->getMessage());
    }
} catch (Exception $e) {
    echo "final: ", $e->getMessage(), "\n";
}
