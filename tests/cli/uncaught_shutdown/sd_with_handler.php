<?php
set_exception_handler(function ($e) { echo "H\n"; });
register_shutdown_function(function () { echo "shutdown\n"; });
throw new RuntimeException('boom');
