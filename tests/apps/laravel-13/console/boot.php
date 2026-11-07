<?php
require __DIR__.'/../vendor/autoload.php';
$app = require __DIR__.'/../bootstrap/app.php';
$app->make(Illuminate\Contracts\Console\Kernel::class)->bootstrap();
function section($n, $f) { echo "== $n\n"; try { $r = $f(); if ($r !== null) echo is_string($r) ? $r : json_encode($r, JSON_PRETTY_PRINT|JSON_UNESCAPED_SLASHES), "\n"; } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), ' @ ', basename($e->getFile()), ':', $e->getLine(), "\n"; } }
