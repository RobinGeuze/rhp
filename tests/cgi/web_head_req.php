<?php header('X-H: 1'); echo str_repeat("body", 100); register_shutdown_function(function(){ echo "sd"; });
