<?php ob_start(function($b){ throw new Exception("in handler"); }); echo "x"; ob_end_flush(); echo "after";
