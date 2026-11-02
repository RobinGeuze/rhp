<?php
header("Content-Type: application/json; charset=utf-8");
register_shutdown_function(function () { echo "]"; });
echo "[1";
exit(3);
