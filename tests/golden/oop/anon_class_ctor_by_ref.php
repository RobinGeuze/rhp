<?php
$a = 1;
$o = new class($a) { public function __construct(&$x) { $x = 2; } };
var_dump($a);
$arr = ['k' => 1];
$o = new class($arr['k']) { public function __construct(&$x) { $x = 3; } };
var_dump($arr);
class P { public $p = 1; function f() { new class($this->p) { public function __construct(&$x) { $x = 4; } }; } }
$p = new P; $p->f(); var_dump($p->p);
// A lazily built exception message (Symfony's AutowiringFailedException):
// the message property is bound by reference into an anonymous class.
class E extends RuntimeException {
    private ?\Closure $messageCallback = null;
    public function __construct(string|\Closure $message = '') {
        $this->messageCallback = $message;
        parent::__construct('', 0, null);
        $this->message = new class($this->message, $this->messageCallback) {
            private string|self $message;
            private ?\Closure $messageCallback;
            public function __construct(&$message, &$messageCallback) {
                $this->message = &$message;
                $this->messageCallback = &$messageCallback;
            }
            public function __toString(): string {
                $messageCallback = $this->messageCallback;
                $this->messageCallback = null;
                try { return $this->message = $messageCallback(); }
                catch (\Throwable $e) { return $this->message = $e->getMessage(); }
            }
        };
    }
}
$c = 0;
$e = new E(function () use (&$c) { $c++; return "hello"; });
var_dump($c, $e->getMessage(), $e->getMessage(), $c);
$e = new E(function () use (&$c) { $c++; throw new Exception('boo'); });
var_dump($e->getMessage(), $e->getMessage(), $c);
// getMessage / getFile / getLine convert like zval_get_string / zval_get_long.
class F extends Exception { function set() { $this->message = 42; $this->line = "7"; } }
$f = new F('x'); $f->set();
var_dump($f->getMessage(), $f->getLine());
