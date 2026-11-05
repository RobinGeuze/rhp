<?php

$config = require __DIR__ . '/.php-cs-fixer.dist.php';

return $config->setParallelConfig(new PhpCsFixer\Runner\Parallel\ParallelConfig(2, 1));
