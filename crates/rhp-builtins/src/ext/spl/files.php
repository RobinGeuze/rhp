
class SplFileInfo implements Stringable {
    protected $__path = ''; protected $__dir = ''; protected $__infoClass = 'SplFileInfo'; protected $__fileClass = 'SplFileObject';
    public function __construct(string $filename) { $this->__setPath($filename); }
    protected function __setPath(string $p): void {
        $len = strlen($p);
        while ($len > 1 && $p[$len - 1] === '/') { $len--; }
        $p = substr($p, 0, $len);
        $this->__path = $p;
        $s = strrpos($p, '/');
        $this->__dir = $s === false ? '' : substr($p, 0, $s);
    }
    // spl_filesystem_object_get_path / _get_pathname: a directory
    // iterator's are its current entry's.
    public function getPath(): string { return $this instanceof DirectoryIterator ? $this->__entryPath() : $this->__dir; }
    public function getFilename(): string {
        $p = $this->getPathname(); $d = $this->getPath(); $dl = strlen($d);
        if ($dl > 0 && $dl < strlen($p)) { return substr($p, $dl + 1); }
        return $p;
    }
    public function getExtension(): string {
        $b = basename($this->getFilename()); $q = strrpos($b, '.');
        return $q === false ? '' : substr($b, $q + 1);
    }
    public function getBasename(string $suffix = ''): string { return basename($this->getFilename(), $suffix); }
    public function getPathname(): string { return $this instanceof DirectoryIterator ? $this->__entryPathname() : $this->__path; }
    // spl_filesystem_object_get_debug_info: a file object adds its mode
    // and CSV control.
    public function __debugInfo(): array {
        $r = __spl_user_props($this) + ["\0SplFileInfo\0pathName" => $this->getPathname(), "\0SplFileInfo\0fileName" => $this->getFilename()];
        if ($this instanceof SplFileObject) { $r += $this->__fileDebug(); }
        return $r;
    }
    public function getPerms(): int|false { return $this->__stat('getPerms', 'fileperms'); }
    public function getInode(): int|false { return $this->__stat('getInode', 'fileinode'); }
    public function getSize(): int|false { return $this->__stat('getSize', 'filesize'); }
    public function getOwner(): int|false { return $this->__stat('getOwner', 'fileowner'); }
    public function getGroup(): int|false { return $this->__stat('getGroup', 'filegroup'); }
    public function getATime(): int|false { return $this->__stat('getATime', 'fileatime'); }
    public function getMTime(): int|false { return $this->__stat('getMTime', 'filemtime'); }
    public function getCTime(): int|false { return $this->__stat('getCTime', 'filectime'); }
    public function getType(): string|false {
        $r = @filetype($this->getPathname());
        if ($r === false) { throw new RuntimeException('SplFileInfo::getType(): Lstat failed for ' . $this->getPathname()); }
        return $r;
    }
    private function __stat(string $m, string $f) {
        $r = @$f($this->getPathname());
        if ($r === false) { throw new RuntimeException("SplFileInfo::$m(): stat failed for " . $this->getPathname()); }
        return $r;
    }
    public function isWritable(): bool { return is_writable($this->getPathname()); }
    public function isReadable(): bool { return is_readable($this->getPathname()); }
    public function isExecutable(): bool { return is_executable($this->getPathname()); }
    public function isFile(): bool { return is_file($this->getPathname()); }
    public function isDir(): bool { return is_dir($this->getPathname()); }
    public function isLink(): bool { return is_link($this->getPathname()); }
    public function getLinkTarget(): string|false {
        $r = @readlink($this->getPathname());
        if ($r === false) { throw new RuntimeException('Unable to read link ' . $this->getPathname() . ', error: ' . (is_link($this->getPathname()) ? 'Unknown error' : 'Invalid argument')); }
        return $r;
    }
    public function getRealPath(): string|false { return __spl_realpath($this->getPathname() === '' ? '.' : $this->getPathname()); }
    public function getFileInfo(?string $class = null): SplFileInfo { $c = $class ?? $this->__infoClass; return new $c($this->getPathname()); }
    public function getPathInfo(?string $class = null): ?SplFileInfo {
        $c = $class ?? $this->__infoClass; $p = $this->getPath();
        return $p === '' ? null : new $c($p);
    }
    public function openFile(string $mode = 'r', bool $useIncludePath = false, $context = null): SplFileObject {
        $c = $this->__fileClass;
        try {
            return new $c($this->getPathname(), $mode, $useIncludePath, $context);
        } catch (RuntimeException $e) {
            $m = $e->getMessage();
            if (str_starts_with($m, 'SplFileObject::__construct(')) { throw new RuntimeException('SplFileInfo::openFile' . substr($m, 26)); }
            throw $e;
        }
    }
    public function setFileClass(string $class = SplFileObject::class): void {
        if (!is_a($class, 'SplFileObject', true)) { throw new TypeError("SplFileInfo::setFileClass(): Argument #1 (\$class) must be a class name derived from SplFileObject, $class given"); }
        $this->__fileClass = $class;
    }
    public function setInfoClass(string $class = SplFileInfo::class): void {
        if (!is_a($class, 'SplFileInfo', true)) { throw new TypeError("SplFileInfo::setInfoClass(): Argument #1 (\$class) must be a class name derived from SplFileInfo, $class given"); }
        $this->__infoClass = $class;
    }
    public function __toString(): string { return $this->getPathname(); }
    #[\Deprecated(since: '8.2')]
    final public function _bad_state_ex(): void { throw new Error('The parent constructor was not called: the object is in an invalid state'); }
}

class DirectoryIterator extends SplFileInfo implements SeekableIterator {
    protected $__entries = []; protected $__pos = 0; protected $__flags = 0;
    public function __construct(string $directory) { $this->__open('DirectoryIterator', $directory, 0); }
    protected function __open(string $cls, string $directory, int $flags): void {
        if ($directory === '') { throw new ValueError("$cls::__construct(): Argument #1 (\$directory) must not be empty"); }
        $this->__flags = $flags;
        $list = @scandir($directory, SCANDIR_SORT_NONE);
        if ($list === false) {
            $e = error_get_last(); $why = 'No such file or directory';
            if ($e && ($q = strrpos($e['message'], ': ')) !== false) { $why = substr($e['message'], $q + 2); }
            throw new UnexpectedValueException("$cls::__construct($directory): Failed to open directory: $why");
        }
        $len = strlen($directory);
        while ($len > 1 && $directory[$len - 1] === '/') { $len--; }
        $this->__dir = substr($directory, 0, $len);
        $this->__entries = $list;
        $this->__pos = 0;
        $this->__skip();
    }
    protected function __skip(): void {
        if ($this->__flags & 4096) {
            while (isset($this->__entries[$this->__pos]) && ($this->__entries[$this->__pos] === '.' || $this->__entries[$this->__pos] === '..')) { $this->__pos++; }
        }
    }
    public function getFilename(): string { return $this->__entryFilename(); }
    protected function __entryFilename(): string { return $this->__entries[$this->__pos] ?? ''; }
    protected function __entryPath(): string { return $this->__dir; }
    protected function __entryPathname(): string {
        $e = $this->__entries[$this->__pos] ?? '';
        if ($e === '') { return ''; }
        return $this->__dir === '' ? $e : $this->__dir . '/' . $e;
    }
    public function getExtension(): string { $b = $this->getFilename(); $q = strrpos($b, '.'); return $q === false ? '' : substr($b, $q + 1); }
    public function getBasename(string $suffix = ''): string { return basename($this->getFilename(), $suffix); }
    public function isDot(): bool { $f = $this->getFilename(); return $f === '.' || $f === '..'; }
    public function rewind(): void { $this->__pos = 0; $this->__skip(); }
    public function valid(): bool { return isset($this->__entries[$this->__pos]); }
    public function key(): mixed { return $this->__pos; }
    public function current(): mixed { return $this; }
    public function next(): void { $this->__pos++; $this->__skip(); }
    public function seek(int $offset): void {
        if ($this->__pos !== $offset) { $this->rewind(); }
        while ($this->__pos < $offset && $this->valid()) { $this->next(); }
        if (!$this->valid()) { throw new OutOfBoundsException("Seek position $offset is out of range"); }
    }
    public function __toString(): string { return $this->getFilename(); }
}

class FilesystemIterator extends DirectoryIterator {
    const CURRENT_MODE_MASK = 240; const CURRENT_AS_PATHNAME = 32; const CURRENT_AS_FILEINFO = 0; const CURRENT_AS_SELF = 16;
    const KEY_MODE_MASK = 3840; const KEY_AS_PATHNAME = 0; const KEY_AS_FILENAME = 256; const FOLLOW_SYMLINKS = 16384;
    const NEW_CURRENT_AND_KEY = 256; const OTHER_MODE_MASK = 28672; const SKIP_DOTS = 4096; const UNIX_PATHS = 8192;
    public function __construct(string $directory, int $flags = FilesystemIterator::KEY_AS_PATHNAME | FilesystemIterator::CURRENT_AS_FILEINFO | FilesystemIterator::SKIP_DOTS) {
        $this->__open('FilesystemIterator', $directory, $flags);
    }
    public function rewind(): void { $this->__pos = 0; $this->__skip(); }
    public function key(): mixed { return ($this->__flags & self::KEY_AS_FILENAME) ? $this->getFilename() : $this->getPathname(); }
    public function current(): mixed {
        if ($this->__flags & self::CURRENT_AS_PATHNAME) { return $this->getPathname(); }
        if ($this->__flags & self::CURRENT_AS_SELF) { return $this; }
        $c = $this->__infoClass;
        return new $c($this->getPathname());
    }
    public function getFlags(): int { return $this->__flags & (self::KEY_MODE_MASK | self::CURRENT_MODE_MASK | self::OTHER_MODE_MASK); }
    public function setFlags(int $flags): void {
        $mask = self::KEY_MODE_MASK | self::CURRENT_MODE_MASK | self::OTHER_MODE_MASK;
        $this->__flags = ($this->__flags & ~$mask) | ($flags & $mask);
    }
}

class RecursiveDirectoryIterator extends FilesystemIterator implements RecursiveIterator {
    protected $__subPath = '';
    public function __construct(string $directory, int $flags = FilesystemIterator::KEY_AS_PATHNAME | FilesystemIterator::CURRENT_AS_FILEINFO) {
        $this->__open('RecursiveDirectoryIterator', $directory, $flags);
    }
    public function hasChildren(bool $allowLinks = false): bool {
        if ($this->isDot()) { return false; }
        $p = $this->getPathname();
        if (!$allowLinks && !($this->__flags & self::FOLLOW_SYMLINKS) && is_link($p)) { return false; }
        return is_dir($p);
    }
    public function getChildren(): RecursiveDirectoryIterator {
        // `new static` so a subclass (e.g. symfony Finder's) keeps its own type.
        $child = new static($this->getPathname(), $this->__flags);
        $child->__subPath = $this->getSubPathname();
        $child->__infoClass = $this->__infoClass; $child->__fileClass = $this->__fileClass;
        return $child;
    }
    public function getSubPath(): string { return $this->__subPath; }
    public function getSubPathname(): string {
        return ($this->__subPath === '' ? '' : $this->__subPath . '/') . $this->getFilename();
    }
}

class GlobIterator extends FilesystemIterator implements Countable {
    public function __construct(string $pattern, int $flags = FilesystemIterator::KEY_AS_PATHNAME | FilesystemIterator::CURRENT_AS_FILEINFO) {
        if ($pattern === '') { throw new ValueError('GlobIterator::__construct(): Argument #1 ($pattern) must not be empty'); }
        $this->__flags = $flags;
        $list = glob($pattern);
        $this->__entries = $list === false ? [] : $list;
        $this->__pos = 0;
        $s = strrpos($pattern, '/');
        $this->__dir = $s === false ? '' : substr($pattern, 0, $s);
    }
    protected function __entryFilename(): string { $e = $this->__entries[$this->__pos] ?? ''; return $e === '' ? '' : basename($e); }
    protected function __entryPathname(): string { return $this->__entries[$this->__pos] ?? ''; }
    protected function __entryPath(): string { $e = $this->__entries[$this->__pos] ?? ''; $s = strrpos($e, '/'); return $s === false ? $this->__dir : substr($e, 0, $s); }
    public function count(): int { return count($this->__entries); }
}

class SplFileObject extends SplFileInfo implements RecursiveIterator, SeekableIterator {
    const DROP_NEW_LINE = 1; const READ_AHEAD = 2; const SKIP_EMPTY = 4; const READ_CSV = 8;
    protected $__fp = null; protected $__line = null; protected $__zval = null; protected $__num = 0;
    protected $__fflags = 0; protected $__maxlen = 0; protected $__mode = 'r'; protected $__fname = '';
    protected $__d = ','; protected $__e = '"'; protected $__esc = '\\'; protected $__escdef = true;
    public function __construct(string $filename, string $mode = 'r', bool $useIncludePath = false, $context = null) {
        $this->__fopen('SplFileObject::__construct', $filename, $mode, $useIncludePath, $context);
    }
    protected function __fopen(string $fn, string $filename, string $mode, bool $inc, $ctx): void {
        if ($this->__fp !== null) { throw new Error('Cannot call constructor twice'); }
        if (is_dir($filename) && !str_contains($filename, '://')) { throw new LogicException('Cannot use SplFileObject with directories'); }
        error_clear_last();
        $fp = $ctx === null ? @fopen($filename, $mode, $inc) : @fopen($filename, $mode, $inc, $ctx);
        if ($fp === false) {
            $e = error_get_last(); $msg = $e ? $e['message'] : "Failed to open stream: No such file or directory";
            if (str_starts_with($msg, 'fopen(')) { $msg = $fn . substr($msg, 5); }
            throw new RuntimeException($msg);
        }
        $this->__fp = $fp; $this->__mode = $mode; $this->__fname = $filename;
        $this->__setPath($filename);
    }
    public function __tmp(string $name): void {
        $this->__fp = fopen($name, 'wb'); $this->__mode = 'wb'; $this->__fname = $name; $this->__path = $name; $this->__dir = '';
    }
    private function __free(): void { $this->__line = null; $this->__zval = null; }
    private function __readEx(bool $silent, int $add, bool $csv): bool {
        $this->__free();
        if (feof($this->__fp)) {
            if (!$silent) { throw new RuntimeException('Cannot read from file ' . $this->__fname); }
            return false;
        }
        $buf = $this->__maxlen > 0 ? fgets($this->__fp, $this->__maxlen + 1) : fgets($this->__fp);
        if ($buf === false) { $this->__line = ''; }
        else {
            if (!$csv && ($this->__fflags & self::DROP_NEW_LINE)) {
                $l = strlen($buf);
                if ($l > 0 && $buf[$l - 1] === "\n") { $l--; if ($l > 0 && $buf[$l - 1] === "\r") { $l--; } $buf = substr($buf, 0, $l); }
            }
            $this->__line = $buf;
        }
        $this->__num += $add;
        return true;
    }
    private function __read(bool $silent, bool $csv): bool { return $this->__readEx($silent, $this->__line !== null ? 1 : 0, $csv); }
    private function __lineEmpty(): bool {
        $l = $this->__line;
        return $l === '' || (($this->__fflags & self::READ_CSV) && ($this->__fflags & self::DROP_NEW_LINE) && ($l === "\n" || $l === "\r\n"));
    }
    private function __readCsv(string $d, string $e, string $esc, bool $silent): bool {
        do {
            if (!$this->__read($silent, true)) { return false; }
        } while ($this->__lineEmpty() && ($this->__fflags & self::SKIP_EMPTY));
        $buf = $this->__line;
        // An enclosure left open continues on the next line(s).
        while ($e !== '' && substr_count($buf, $e) % 2 === 1 && !feof($this->__fp)) {
            $more = fgets($this->__fp); if ($more === false) { break; } $buf .= $more;
        }
        if (str_ends_with($buf, "\r\n")) { $buf = substr($buf, 0, -2); } elseif (str_ends_with($buf, "\n") || str_ends_with($buf, "\r")) { $buf = substr($buf, 0, -1); }
        $this->__zval = str_getcsv($buf, $d, $e, $esc);
        return true;
    }
    private function __readLineEx(bool $silent): bool {
        if ($this->__fflags & self::READ_CSV) { return $this->__readCsv($this->__d, $this->__e, $this->__esc, $silent); }
        return $this->__read($silent, false);
    }
    private function __readLine(bool $silent): bool {
        $r = $this->__readLineEx($silent);
        while (($this->__fflags & self::SKIP_EMPTY) && $r && $this->__lineEmpty()) { $this->__free(); $r = $this->__readLineEx($silent); }
        return $r;
    }
    public function rewind(): void {
        if (!@rewind($this->__fp)) { throw new RuntimeException('Cannot rewind file ' . $this->__fname); }
        $this->__free(); $this->__num = 0;
        if ($this->__fflags & self::READ_AHEAD) { $this->__readLine(true); }
    }
    public function eof(): bool { return feof($this->__fp); }
    public function valid(): bool {
        if ($this->__fflags & self::READ_AHEAD) { return $this->__line !== null || $this->__zval !== null; }
        return !feof($this->__fp);
    }
    public function fgets(): string { $this->__readEx(false, 1, false); return $this->__line; }
    public function current(): string|array|false {
        if ($this->__line === null && $this->__zval === null) { $this->__readLine(true); }
        if ($this->__line !== null && (!($this->__fflags & self::READ_CSV) || $this->__zval === null)) { return $this->__line; }
        if ($this->__zval !== null) { return $this->__zval; }
        return false;
    }
    public function key(): int { return $this->__num; }
    public function next(): void {
        $this->__free();
        if ($this->__fflags & self::READ_AHEAD) { $this->__readLine(true); }
        $this->__num++;
    }
    public function setFlags(int $flags): void { $this->__fflags = $flags; }
    public function getFlags(): int { return $this->__fflags; }
    public function setMaxLineLen(int $maxLength): void {
        if ($maxLength < 0) { throw new ValueError('SplFileObject::setMaxLineLen(): Argument #1 ($maxLength) must be greater than or equal to 0'); }
        $this->__maxlen = $maxLength;
    }
    public function getMaxLineLen(): int { return $this->__maxlen; }
    public function hasChildren(): bool { return false; }
    public function getChildren(): ?RecursiveIterator { return null; }
    private function __csvArgs(string $m, ?string $d, ?string $e, ?string $esc, int $base): array {
        if ($d !== null && strlen($d) !== 1) { throw new ValueError("SplFileObject::$m(): Argument #$base (\$separator) must be a single character"); }
        if ($e !== null && strlen($e) !== 1) { throw new ValueError("SplFileObject::$m(): Argument #" . ($base + 1) . " (\$enclosure) must be a single character"); }
        if ($esc === null) {
            if ($this->__escdef) { __spl_deprecated("SplFileObject::$m(): the \$escape parameter must be provided, as its default value will change, either explicitly or via SplFileObject::setCsvControl()"); }
            $esc = $this->__esc;
        } elseif (strlen($esc) > 1) {
            throw new ValueError("SplFileObject::$m(): Argument #" . ($base + 2) . ' ($escape) must be empty or a single character');
        }
        return [$d ?? $this->__d, $e ?? $this->__e, $esc];
    }
    public function fgetcsv(?string $separator = null, ?string $enclosure = null, ?string $escape = null): array|false {
        [$d, $e, $esc] = $this->__csvArgs('fgetcsv', $separator, $enclosure, $escape, 1);
        if (!$this->__readCsv($d, $e, $esc, true)) { return false; }
        return $this->__zval;
    }
    public function fputcsv(array $fields, ?string $separator = null, ?string $enclosure = null, ?string $escape = null, string $eol = "\n"): int|false {
        [$d, $e, $esc] = $this->__csvArgs('fputcsv', $separator, $enclosure, $escape, 2);
        return fputcsv($this->__fp, $fields, $d, $e, $esc, $eol);
    }
    public function setCsvControl(string $separator = ',', string $enclosure = '"', ?string $escape = null): void {
        if (strlen($separator) !== 1) { throw new ValueError('SplFileObject::setCsvControl(): Argument #1 ($separator) must be a single character'); }
        if (strlen($enclosure) !== 1) { throw new ValueError('SplFileObject::setCsvControl(): Argument #2 ($enclosure) must be a single character'); }
        if ($escape !== null) {
            if (strlen($escape) > 1) { throw new ValueError('SplFileObject::setCsvControl(): Argument #3 ($escape) must be empty or a single character'); }
            $this->__escdef = false; $this->__esc = $escape;
        } else { $this->__esc = '\\'; }
        $this->__d = $separator; $this->__e = $enclosure;
    }
    public function getCsvControl(): array { return [$this->__d, $this->__e, $this->__esc]; }
    protected function __fileDebug(): array {
        return ["\0SplFileObject\0openMode" => $this->__mode, "\0SplFileObject\0delimiter" => $this->__d, "\0SplFileObject\0enclosure" => $this->__e];
    }
    public function flock(int $operation, &$wouldBlock = null): bool { return flock($this->__fp, $operation, $wouldBlock); }
    public function fflush(): bool { return fflush($this->__fp); }
    public function ftell(): int|false { return ftell($this->__fp); }
    public function fseek(int $offset, int $whence = SEEK_SET): int { $this->__free(); return fseek($this->__fp, $offset, $whence); }
    public function fgetc(): string|false {
        $this->__free();
        $c = fgetc($this->__fp);
        if ($c === false) { return false; }
        if ($c === "\n") { $this->__num++; }
        return $c;
    }
    public function fpassthru(): int { return fpassthru($this->__fp); }
    public function fscanf(string $format, mixed &...$vars): array|int|null {
        $this->__read(false, false);
        return sscanf($this->__line, $format);
    }
    public function fwrite(string $data, int $length = 0): int|false {
        if (func_num_args() > 1) { $data = $length >= 0 ? substr($data, 0, $length) : ''; }
        if ($data === '') { return 0; }
        return fwrite($this->__fp, $data);
    }
    public function fread(int $length): string|false {
        if ($length <= 0) { throw new ValueError('SplFileObject::fread(): Argument #1 ($length) must be greater than 0'); }
        return fread($this->__fp, $length);
    }
    public function fstat(): array { return fstat($this->__fp); }
    public function ftruncate(int $size): bool {
        if ($size < 0) { throw new ValueError('SplFileObject::ftruncate(): Argument #1 ($size) must be greater than or equal to 0'); }
        return ftruncate($this->__fp, $size);
    }
    public function seek(int $line): void {
        if ($line < 0) { throw new ValueError('SplFileObject::seek(): Argument #1 ($line) must be greater than or equal to 0'); }
        $this->rewind();
        for ($i = 0; $i < $line; $i++) {
            if (!$this->__readLine(true)) { return; }
        }
        if ($line > 0 && !($this->__fflags & self::READ_AHEAD)) { $this->__num++; $this->__free(); }
    }
    public function getCurrentLine(): string { return $this->fgets(); }
    public function __toString(): string {
        if ($this->__line === null) { $this->__readLine(false); }
        return $this->__line;
    }
}

class SplTempFileObject extends SplFileObject {
    public function __construct(int $maxMemory = 2097152) {
        if ($maxMemory < 0) { $name = 'php://memory'; }
        elseif (func_num_args() > 0) { $name = "php://temp/maxmemory:$maxMemory"; }
        else { $name = 'php://temp'; }
        $this->__tmp($name);
    }
}
