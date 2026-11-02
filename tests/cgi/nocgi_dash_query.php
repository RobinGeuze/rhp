<?php
// headers_list() after -q differs between php builds (8.5.8 keeps
// X-Powered-By, Ubuntu 8.5.12 does not); quiet_mode covers -q.
var_dump($_GET, $argv, getcwd() === __DIR__);
