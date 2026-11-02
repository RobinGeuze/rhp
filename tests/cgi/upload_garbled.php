<?php
require __DIR__ . '/upload_show.inc';
var_dump($_POST, upload_files($_FILES), file_get_contents('php://input'));
