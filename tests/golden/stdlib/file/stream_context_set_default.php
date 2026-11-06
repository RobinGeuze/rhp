<?php
// stream_context_set_default() applies options to the default context and
// returns it; stream_context_get_default() sees them.
$d = stream_context_set_default(['http' => ['method' => 'HEAD', 'timeout' => 5]]);
var_dump(get_resource_type($d));
var_dump($d === stream_context_get_default());
var_dump(stream_context_get_options(stream_context_get_default()));
stream_context_set_default(['http' => ['method' => 'GET'], 'ftp' => ['overwrite' => true]]);
var_dump(stream_context_get_options($d));
try {
    stream_context_set_default(['http' => 1]);
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}
try {
    stream_context_set_default('x');
} catch (TypeError $e) {
    echo $e->getMessage(), "\n";
}
