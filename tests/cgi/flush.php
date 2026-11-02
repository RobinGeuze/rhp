<?php
echo "a";
flush();
header("X-Too-Late: 1");
echo "b\n";
