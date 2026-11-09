<?php
set_exception_handler(function ($e) { echo "handled ", $e->getMessage(), "\n"; });
echo "main\n";
throw new Exception("e");
