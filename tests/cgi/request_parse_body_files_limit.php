<?php
require __DIR__ . '/upload_show.inc';
var_dump($_POST, upload_files($_FILES));
try {
    [$post, $files] = request_parse_body(['max_file_uploads' => 0]);
    var_dump($post, upload_files($files));
} catch (Throwable $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
var_dump(file_get_contents('php://input'));
try {
    [$post, $files] = request_parse_body();
    var_dump($post, upload_files($files));
} catch (Throwable $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
