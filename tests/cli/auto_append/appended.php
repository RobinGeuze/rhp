<?php
register_shutdown_function(function () { echo "shutdown\n"; });
$x = "set by the script";
echo "main\n";
return 4;
