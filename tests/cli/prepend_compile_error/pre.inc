<?php
echo "prepend\n";
register_shutdown_function(function () { echo "shutdown\n"; });
