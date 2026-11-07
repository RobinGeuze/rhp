<?php
// break/continue through finally blocks: each foreach over a temporary is
// freed once the finally inside it ran, before the next finally out.
class D { function __construct(public $n) {} function __destruct() { echo "~{$this->n}\n"; } }
function f() {
    foreach (new D("outer") as $_) {
        try {
            foreach (new D("mid") as $__) {
                try {
                    foreach (new D("inner") as $___) {
                        try {
                            continue 3;
                        } finally { echo "f1\n"; }
                    }
                } finally { echo "f2\n"; }
            }
        } finally { echo "f3\n"; }
    }
    echo "end\n";
}
f();
function g() {
    for ($i = 0; $i < 2; $i++) {
        try {
            foreach (new D("g$i") as $_) {
                try { break 2; } finally { echo "gf1\n"; }
            }
        } finally { echo "gf2\n"; }
    }
    echo "gend\n";
}
g();
