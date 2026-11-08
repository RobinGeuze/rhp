<?php
// Declines static files and sub/: the server then serves the file, or runs
// the script in this same request.
function router_fn() {}
$fromrouter = 1;
register_shutdown_function(function () { error_log("shutdown ran"); });
$path = parse_url($_SERVER['REQUEST_URI'], PHP_URL_PATH);
if (str_starts_with($path, '/sub/') || $path === '/a.css' || $path === '/gone.txt') {
    return false;
}
echo "routed ", $_SERVER['REQUEST_URI'], " ", $_SERVER['SCRIPT_NAME'], "\n";
