<?php
use Rector\Config\RectorConfig;
return RectorConfig::configure()
    ->withPaths([__DIR__ . '/vendor/symfony/console'])
    ->withCache(__DIR__ . '/var/rector-big-parallel')
    ->withPhpSets(php84: true)
    ->withPreparedSets(deadCode: true, codeQuality: true, typeDeclarations: true, codingStyle: true, earlyReturn: true)
    ->withParallel(300, 4, 10);
