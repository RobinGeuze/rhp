<?php ob_start(function($b){ return strtoupper($b); }); echo "buffered "; undefined_fn();
