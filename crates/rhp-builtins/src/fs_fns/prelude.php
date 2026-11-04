
final class Directory {
    public readonly string $path;
    public readonly mixed $handle;
    public function close(): void {
        if (func_num_args()) throw new ArgumentCountError('Directory::close() expects exactly 0 arguments, ' . func_num_args() . ' given');
        __directory_op($this->handle, "close");
    }
    public function rewind(): void {
        if (func_num_args()) throw new ArgumentCountError('Directory::rewind() expects exactly 0 arguments, ' . func_num_args() . ' given');
        __directory_op($this->handle, "rewind");
    }
    public function read(): string|false {
        if (func_num_args()) throw new ArgumentCountError('Directory::read() expects exactly 0 arguments, ' . func_num_args() . ' given');
        return __directory_op($this->handle, "read");
    }
}
