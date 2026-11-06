<?php

$finder = (new PhpCsFixer\Finder())->in([__DIR__ . '/src', __DIR__ . '/tests']);

return (new PhpCsFixer\Config())
    ->setRules(['@PSR12' => true, 'array_syntax' => ['syntax' => 'short']])
    ->setCacheFile(__DIR__ . '/var/php-cs-fixer.cache')
    ->setParallelConfig(new PhpCsFixer\Runner\Parallel\ParallelConfig(1))
    ->setFinder($finder);
