<?php echo "a"; flush(); header('X-No: 1'); echo "b"; ob_implicit_flush(true); echo "c"; var_dump(headers_sent($f,$l), $l);
