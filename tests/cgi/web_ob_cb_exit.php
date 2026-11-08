<?php ob_start(function($b){ return "<".$b.">"; }); echo "x"; exit("bye");
