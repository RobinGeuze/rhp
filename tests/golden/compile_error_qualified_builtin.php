<?php
// A fully-qualified builtin type (`\Mixed`, here a property type) is the
// compile error "Type declaration 'mixed' must be unqualified"; qualified
// class names are fine.
namespace Foo {
    class Bar {}
}

namespace {
    ob_start(function ($s) { return str_replace(__FILE__, 'F', $s); });
    function ok(\Foo\Bar $a, ?\Countable $b = null): \Traversable|\ArrayAccess|null { return null; }
    var_dump(ok(new \Foo\Bar));
    eval("namespace N;\n\nclass P {\n  public int \$a = 1;\n  public ?\\Mixed \$x;\n}");
    echo "not reached\n";
}
