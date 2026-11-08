<?php header("X-A: a\r\nX-B: b"); header("X-C: c\n"); header("X-D: d\0e"); header("NoColon"); header(" X-Space: 1"); header("X-Empty:"); header(""); var_dump(headers_list());
