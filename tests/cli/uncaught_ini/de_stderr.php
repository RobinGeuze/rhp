<?php
ini_set('display_errors', 'stderr');
echo "a\n";
throw new Exception('to stderr');
