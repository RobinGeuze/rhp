<?php
// The parser folds parentheses, concatenation and plain heredocs.
declare(ticks=(1));
declare(ticks="1" . "0");
declare(ticks=<<<X
5
X);
echo "ran\n";
