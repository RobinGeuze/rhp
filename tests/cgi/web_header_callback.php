<?php header_register_callback(function(){ header('X-CB: 1'); header_remove('X-Powered-By'); setcookie('cb','1'); }); echo "body";
