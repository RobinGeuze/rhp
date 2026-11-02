<?php
var_dump(setcookie("plain", "value"));
setcookie("spaced", "a b+c/d~");
setcookie("empty", "");
setcookie("deleted");
setcookie("past", "v", 1, "/p", "example.com", true, true);
setcookie("opts", "v", ["expires" => 1, "path" => "/", "domain" => ".d.com", "secure" => true, "httponly" => true, "samesite" => "Strict"]);
setcookie("part", "v", ["secure" => true, "partitioned" => true]);
setrawcookie("raw", "a%20b");
foreach ([
    fn() => setcookie("", "v"),
    fn() => setcookie("a=b", "v"),
    fn() => setrawcookie("r", "a b"),
    fn() => setcookie("o", "v", ["bogus" => 1]),
    fn() => setcookie("o", "v", [1 => 1]),
    fn() => setcookie("o", "v", ["path" => "a;b"]),
    fn() => setcookie("o", "v", ["partitioned" => true]),
    fn() => setcookie("o", "v", ["expires" => 253402300800]),
    fn() => setcookie("o", "v", [], "/"),
] as $f) {
    try { $f(); } catch (\Throwable $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}
echo json_encode(headers_list()), "\n";
