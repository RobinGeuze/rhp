<?php
register_shutdown_function(function () { header("X-Shutdown: 1"); echo "shutdown\n"; });
header("X-Main: 1");
