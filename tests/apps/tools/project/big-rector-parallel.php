<?php
// Worker timeout 900 s: rhp runs these several times slower than php.
use Rector\Config\RectorConfig;
return RectorConfig::configure()
    ->withPaths([__DIR__ . '/vendor/symfony/console'])
    ->withCache(__DIR__ . '/var/rector-big-parallel')
    ->withPhpSets(php84: true)
    ->withPreparedSets(deadCode: true, codeQuality: true, typeDeclarations: true, codingStyle: true, earlyReturn: true)
    ->withParallel(900, 4, 10);
