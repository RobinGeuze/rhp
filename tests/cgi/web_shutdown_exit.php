<?php
register_shutdown_function(function(){ echo "s1\n"; register_shutdown_function(function(){ echo "nested\n"; }); exit(3); });
register_shutdown_function(function(){ echo "s2 never\n"; });
class D { function __destruct(){ echo "destruct\n"; } }
$d = new D;
echo "main\n";
