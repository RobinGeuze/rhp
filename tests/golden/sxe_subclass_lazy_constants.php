<?php
// simplexml creates an object of a user subclass without evaluating the
// class's constant expressions: an autoloader exception surfaces only where
// one is used, not inside simplexml_load_string().
spl_autoload_register(function ($c) {
    echo "autoload $c\n";
    throw new Exception("autoload $c");
});
class MyEl extends SimpleXMLElement {
    const C = Missing::X;
    public static $s = Missing::Y;
    public function hi() { return "hi"; }
}
$x = simplexml_load_string('<a><b>t</b></a>', 'MyEl');
echo get_class($x), " ", $x->hi(), " ", $x->b, "\n";
try {
    echo MyEl::C, "\n";
} catch (Exception $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
try {
    echo MyEl::$s, "\n";
} catch (Exception $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
