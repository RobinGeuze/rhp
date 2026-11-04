<?php
echo gettype(42), "\n";
echo gettype(1.5), "\n";
echo gettype("s"), "\n";
echo gettype(true), "\n";
echo gettype(null), "\n";
echo gettype([]), "\n";
echo get_debug_type(42), "\n";
echo get_debug_type(1.5), "\n";
echo get_debug_type(true), "\n";
echo get_debug_type(null), "\n";