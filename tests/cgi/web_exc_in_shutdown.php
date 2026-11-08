<?php register_shutdown_function(function(){ throw new Exception("in shutdown"); }); echo "body\n";
