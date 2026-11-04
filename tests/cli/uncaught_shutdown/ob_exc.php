<?php
ob_start();
echo "buffered";
throw new Exception('x');
