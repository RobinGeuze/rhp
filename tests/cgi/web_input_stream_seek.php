<?php $f = fopen('php://input', 'r'); echo fread($f, 3); rewind($f); echo stream_get_contents($f), "\n"; var_dump(stream_get_meta_data($f)['seekable'], fstat($f)['size'] ?? null);
