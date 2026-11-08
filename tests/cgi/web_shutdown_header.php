<?php
register_shutdown_function(function(){ header('X-From-Shutdown: 1'); http_response_code(418); echo "in shutdown\n"; });
