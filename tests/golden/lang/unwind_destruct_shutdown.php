<?php
// A destructor run at shutdown throws, and unwinding it destructs a local
// whose destructor throws too: with no php frame below, php reports the
// second exception on its own (OSS-Fuzz #456317305).
ob_start(function ($s) { return str_replace(__FILE__, 'F', $s); });
class C {
    public $n;
    public function __destruct() {
        static $again = true;
        echo "dtor {$this->n}\n";
        if ($again) {
            $again = false;
            $c = new C; $c->n = "inner";
        }
        throw new Exception("from {$this->n}");
    }
}
register_shutdown_function(function () { echo "shutdown fn\n"; });
$c = new C; $c->n = "main";
echo "end\n";
