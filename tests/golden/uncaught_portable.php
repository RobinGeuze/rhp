<?php
// The uncaught-throwable report goes through the output buffers (flushed at
// shutdown) and is followed by the shutdown functions. The exception pins its
// own file so the report is portable.
class PortableException extends Exception {
    public function __construct(string $m, ?Throwable $prev = null) {
        parent::__construct($m, 7, $prev);
        $this->file = 'portable.php';
        $this->line = 42;
    }
    public function __toString(): string { return static::class . ': ' . $this->getMessage(); }
}
register_shutdown_function(function () { echo "shutdown 1, level ", ob_get_level(), "\n"; });
register_shutdown_function(function () { echo "shutdown 2\n"; });
ob_start();
echo "buffered before the throw\n";
throw new PortableException('boom', new LogicException('inner'));
