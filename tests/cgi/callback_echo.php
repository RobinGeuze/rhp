<?php
header("HTTP/1.1 418 Teapot");
header_register_callback(function () { echo "[cb]"; header("X-Late: 1"); });
echo "main\n";
