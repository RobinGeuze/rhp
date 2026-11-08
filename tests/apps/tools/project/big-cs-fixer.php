<?php
$finder = (new PhpCsFixer\Finder())->in([__DIR__ . '/vendor/symfony/console/Helper', __DIR__ . '/vendor/symfony/console/Formatter', __DIR__ . '/vendor/symfony/console/Output', __DIR__ . '/vendor/symfony/console/Input']);
return (new PhpCsFixer\Config())
    ->setRiskyAllowed(true)
    ->setRules(['@Symfony' => true, '@Symfony:risky' => true, '@PER-CS' => true, '@PHP8x4Migration' => true])
    ->setCacheFile(__DIR__ . '/var/cs-big.cache')
    ->setUsingCache(false)
    ->setParallelConfig(new PhpCsFixer\Runner\Parallel\ParallelConfig(1))
    ->setFinder($finder);
