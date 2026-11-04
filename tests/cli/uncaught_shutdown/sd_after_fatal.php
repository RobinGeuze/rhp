<?php
register_shutdown_function(function () { echo "shutdown\n"; });
throw new RuntimeException('boom');
