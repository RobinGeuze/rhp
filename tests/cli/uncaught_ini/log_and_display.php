<?php
ini_set('log_errors', '1');
ob_start();
echo "a\n";
throw new Exception('both');
