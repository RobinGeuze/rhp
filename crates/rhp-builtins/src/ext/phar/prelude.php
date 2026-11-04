
class PharException extends Exception {}
class Phar {
    const NONE = 0;
    const COMPRESSED = 61440;
    const GZ = 4096;
    const BZ2 = 8192;
    const SAME = 0;
    const PHAR = 1;
    const TAR = 2;
    const ZIP = 3;
    const MD5 = 1;
    const SHA1 = 2;
    const SHA256 = 3;
    const SHA512 = 4;
    const OPENSSL = 16;
    final public static function mapPhar($alias = null, $dataoffset = 0) { return __phar_mapphar($alias); }
    final public static function loadPhar($filename, $alias = null) { return __phar_loadphar($filename, $alias); }
    final public static function running($retphar = true) { return __phar_running($retphar); }
    final public static function canCompress($type = 0) { return $type === self::GZ || $type === self::BZ2 || $type === 0; }
    final public static function isValidPharFilename($filename, $executable = true) { return true; }
    final public static function interceptFileFuncs() {}
    final public static function createDefaultStub($index = null, $webindex = null) { return ""; }
}
class PharFileInfo extends SplFileInfo {
    private $__c = ''; private $__d = false; private $__m = 0; private $__mode = 0666; private $__arc = '';
    public function __construct(string $filename) { parent::__construct($filename); }
    public static function __make(string $path, string $arc, array $e): PharFileInfo {
        $f = new PharFileInfo($path);
        $f->__c = $e['content']; $f->__d = $e['dir']; $f->__m = $e['mtime']; $f->__mode = $e['mode']; $f->__arc = $arc;
        return $f;
    }
    public function getContent(): string {
        if ($this->__d) {
            throw new BadMethodCallException('phar error: Cannot retrieve contents, "' . substr($this->__path, strlen('phar://' . $this->__arc) + 1) . '" in phar "' . $this->__arc . '" is a directory');
        }
        return $this->__c;
    }
    public function isDir(): bool { return $this->__d; }
    public function isFile(): bool { return !$this->__d; }
    public function isLink(): bool { return false; }
    public function isReadable(): bool { return true; }
    public function isWritable(): bool { return true; }
    public function getSize(): int|false { return strlen($this->__c); }
    public function getMTime(): int|false { return $this->__m; }
    public function getCTime(): int|false { return $this->__m; }
    public function getATime(): int|false { return $this->__m; }
    public function getPerms(): int|false { return ($this->__d ? 040000 : 0100000) | $this->__mode; }
    public function getType(): string|false { return $this->__d ? 'dir' : 'file'; }
    public function getCompressedSize(): int { return strlen($this->__c); }
    public function getCRC32(): int { return crc32($this->__c); }
    public function isCRCChecked(): bool { return true; }
    public function isCompressed(int $compression = 9021976): bool { return false; }
    public function hasMetadata(): bool { return false; }
    public function getMetadata(array $unserializeOptions = []): mixed { return null; }
    public function getRealPath(): string|false { return false; }
}
class PharData extends RecursiveDirectoryIterator implements Countable, ArrayAccess {
    private $__file; private $__ents = []; private $__comp = 0; private $__fmt = 2;
    public function __construct(string $filename, int $flags = FilesystemIterator::SKIP_DOTS | FilesystemIterator::UNIX_PATHS, ?string $alias = null, int $format = 0) {
        $this->__flags = $flags;
        $dir = realpath(dirname($filename));
        $this->__file = ($dir === false ? dirname($filename) : $dir) . '/' . basename($filename);
        $lower = strtolower($this->__file);
        if (is_file($filename) && filesize($filename) > 0) {
            if (str_ends_with($lower, '.zip') || $format === Phar::ZIP) {
                throw new UnexpectedValueException('zip-based phar "' . $this->__file . '" is not supported');
            }
            $r = __phardata_load($filename);
            if ($r === false) {
                throw new UnexpectedValueException('internal corruption of phar "' . $this->__file . '" (__HALT_COMPILER(); not found)');
            }
            $this->__comp = $r['comp'];
            foreach ($r['entries'] as $e) { $this->__ents[$e['name']] = $e; }
        } else {
            if (str_ends_with($lower, '.zip') || $format === Phar::ZIP) {
                throw new UnexpectedValueException('zip-based phar "' . $this->__file . '" is not supported');
            }
            if (preg_match('/\.(tar\.gz|tgz)$/', $lower)) { $this->__comp = Phar::GZ; }
            elseif (preg_match('/\.(tar\.bz2|tbz2?)$/', $lower)) { $this->__comp = Phar::BZ2; }
        }
        $this->__dir = 'phar://' . $this->__file;
        $this->__listTop();
    }
    private function __listTop(): void {
        $top = [];
        foreach ($this->__ents as $name => $e) {
            $first = explode('/', $name, 2)[0];
            $top[$first] = true;
        }
        $this->__entries = array_keys($top);
        sort($this->__entries, SORT_STRING);
        $this->__pos = 0;
    }
    private function __flush(): void {
        $r = __phardata_save($this->__file, array_values($this->__ents), $this->__comp);
        if ($r !== true) { throw new PharException($r); }
        $this->__listTop();
    }
    private function __entry(string $name): ?array {
        $name = trim(str_replace('\\', '/', $name), '/');
        if (isset($this->__ents[$name])) { return $this->__ents[$name]; }
        $pfx = $name . '/';
        foreach ($this->__ents as $n => $e) {
            if (str_starts_with($n, $pfx)) { return ['name' => $name, 'dir' => true, 'content' => '', 'mtime' => 0, 'mode' => 0777]; }
        }
        return null;
    }
    private function __info(string $name): PharFileInfo {
        $e = $this->__entry($name);
        return PharFileInfo::__make($this->__dir . '/' . $e['name'], $this->__file, $e);
    }
    // php parses $mode and ignores it.
    public function count(int $mode = COUNT_NORMAL): int { return count($this->__ents); }
    public function offsetExists($localName): bool { return $this->__entry((string) $localName) !== null; }
    public function offsetGet($localName): PharFileInfo {
        if ($this->__entry((string) $localName) === null) {
            throw new BadMethodCallException('Entry ' . $localName . ' does not exist');
        }
        return $this->__info((string) $localName);
    }
    public function offsetSet($localName, $value): void { $this->addFromString((string) $localName, is_resource($value) ? stream_get_contents($value) : (string) $value); }
    public function offsetUnset($localName): void { $this->delete((string) $localName); }
    public function delete(string $localName): true {
        $n = trim($localName, '/');
        if (!isset($this->__ents[$n])) { throw new BadMethodCallException('Entry ' . $localName . ' does not exist and cannot be deleted'); }
        unset($this->__ents[$n]);
        $this->__flush();
        return true;
    }
    private function __add(string $local, string $content, int $mode): void {
        $local = ltrim(str_replace('\\', '/', $local), '/');
        unset($this->__ents[$local]);
        $this->__ents[$local] = ['name' => $local, 'dir' => false, 'content' => $content, 'mtime' => time(), 'mode' => $mode];
    }
    public function addFromString(string $localName, string $contents): void { $this->__add($localName, $contents, 0666 & ~umask()); $this->__flush(); }
    public function addFile(string $filename, ?string $localName = null): void {
        $c = @file_get_contents($filename);
        if ($c === false) { throw new RuntimeException('phar error: unable to open file "' . $filename . '" to add to phar archive'); }
        $this->__add($localName ?? $filename, $c, @fileperms($filename) & 0777);
        $this->__flush();
    }
    public function addEmptyDir(string $directory): void {
        $d = trim(str_replace('\\', '/', $directory), '/');
        if (!isset($this->__ents[$d])) {
            $this->__ents[$d] = ['name' => $d, 'dir' => true, 'content' => '', 'mtime' => time(), 'mode' => 0777];
        }
        $this->__flush();
    }
    public function buildFromIterator(Traversable $iterator, ?string $baseDirectory = null): array {
        $map = [];
        foreach ($iterator as $key => $value) {
            if ($value instanceof SplFileInfo) {
                if ($value->isDir()) { continue; }
                $fname = $value->getPathname();
            } elseif (is_string($value)) {
                $fname = $value;
            } else {
                throw new UnexpectedValueException('Iterator ' . get_class($iterator) . ' returns an SplFileInfo object, so base directory must be specified');
            }
            if ($baseDirectory !== null && $baseDirectory !== '') {
                if (!str_starts_with($fname, $baseDirectory)) {
                    throw new UnexpectedValueException('Iterator ' . get_class($iterator) . ' returned a path "' . $fname . '" that is not in the base directory "' . $baseDirectory . '"');
                }
                $local = ltrim(substr($fname, strlen($baseDirectory)), '/\\');
            } else {
                $local = is_string($key) ? $key : $fname;
            }
            $c = @file_get_contents($fname);
            if ($c === false) { throw new UnexpectedValueException('Iterator ' . get_class($iterator) . ' returned a file that could not be opened "' . $fname . '"'); }
            $this->__add($local, $c, @fileperms($fname) & 0777);
            $map[$local] = $fname;
        }
        if ($map) { $this->__flush(); }
        return $map;
    }
    public function buildFromDirectory(string $directory, string $pattern = ''): array {
        $it = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($directory, FilesystemIterator::SKIP_DOTS));
        $files = [];
        foreach ($it as $f) {
            if ($pattern !== '' && !preg_match($pattern, $f->getPathname())) { continue; }
            $files[$f->getPathname()] = $f;
        }
        ksort($files);
        return $this->buildFromIterator(new ArrayIterator($files), $directory);
    }
    public function compress(int $compression, ?string $extension = null): ?PharData {
        $base = basename($this->__file);
        foreach (['.phar.tar.gz', '.phar.tar.bz2', '.phar.tar', '.phar.zip', '.phar.gz', '.phar.bz2', '.tar.gz', '.tar.bz2', '.tgz', '.tbz', '.tar', '.zip', '.phar'] as $ext) {
            if (strlen($base) > strlen($ext) && str_ends_with($base, $ext)) { $base = substr($base, 0, -strlen($ext)); break; }
        }
        $ext = $extension ?? ($compression === Phar::GZ ? 'tar.gz' : ($compression === Phar::BZ2 ? 'tar.bz2' : 'tar'));
        $new = dirname($this->__file) . '/' . $base . '.' . ltrim($ext, '.');
        if (file_exists($new)) {
            throw new BadMethodCallException('Unable to add newly converted phar "' . $new . '" to the list of phars, a phar with that name already exists');
        }
        $r = __phardata_save($new, array_values($this->__ents), $compression);
        if ($r !== true) { throw new BadMethodCallException($r); }
        return new PharData($new);
    }
    public function decompress(?string $extension = null): ?PharData { return $this->compress(Phar::NONE, $extension); }
    public function extractTo(string $directory, array|string|null $files = null, bool $overwrite = false): bool {
        if ($directory === '') { throw new InvalidArgumentException('Invalid argument, extraction path must be non-zero length'); }
        if (is_string($files)) { $files = [$files]; }
        if (!is_dir($directory) && !@mkdir($directory, 0777, true)) {
            throw new RuntimeException('Unable to create path "' . $directory . '" for extraction');
        }
        foreach ($this->__ents as $name => $e) {
            if ($files !== null) {
                $hit = false;
                foreach ($files as $f) { $f = trim($f, '/'); if ($f === $name || str_starts_with($name, $f . '/')) { $hit = true; break; } }
                if (!$hit) { continue; }
            }
            $parts = [];
            foreach (explode('/', $name) as $p) {
                if ($p === '' || $p === '.') { continue; }
                if ($p === '..') { array_pop($parts); continue; }
                $parts[] = $p;
            }
            if (!$parts) { continue; }
            $full = rtrim($directory, '/') . '/' . implode('/', $parts);
            if (!$overwrite && file_exists($full)) {
                throw new PharException('Extraction from phar "' . $this->__file . '" failed: Cannot extract "' . $name . '" to "' . $full . '", path already exists');
            }
            // phar_extract_file creates the entry's *parent* directory (with
            // the entry's mode for a directory entry), never the entry itself.
            $parent = dirname($full);
            if ($e['dir']) {
                if (!file_exists($parent) && !@mkdir($parent, $e['mode'], true)) {
                    throw new PharException('Extraction from phar "' . $this->__file . '" failed: Cannot extract "' . $name . '", could not create directory "' . $parent . '"');
                }
                continue;
            }
            if (!file_exists($parent) && !@mkdir($parent, 0777, true)) {
                throw new PharException('Extraction from phar "' . $this->__file . '" failed: Cannot extract "' . $name . '", could not create directory "' . $parent . '"');
            }
            if (@file_put_contents($full, $e['content']) === false) {
                throw new PharException('Extraction from phar "' . $this->__file . '" failed: Cannot extract "' . $name . '", could not open for writing "' . $full . '"');
            }
            @chmod($full, $e['mode']);
        }
        return true;
    }
    public function isFileFormat(int $format): bool { return $format === Phar::TAR; }
    public function getPath(): string { return $this->__file; }
    public function getPathname(): string { return $this->__dir . '/' . $this->getFilename(); }
    public function isCompressed(): int|false { return $this->__comp === 0 ? false : $this->__comp; }
    public function isWritable(): bool { return true; }
    public function isDir(): bool { $e = $this->__entry($this->getFilename()); return $e !== null && $e['dir']; }
    public function isFile(): bool { $e = $this->__entry($this->getFilename()); return $e !== null && !$e['dir']; }
    public function hasChildren(bool $allowLinks = false): bool { return $this->isDir(); }
    public function current(): mixed {
        if ($this->__flags & self::CURRENT_AS_PATHNAME) { return $this->getPathname(); }
        if ($this->__flags & self::CURRENT_AS_SELF) { return $this; }
        return $this->__info($this->getFilename());
    }
    public function valid(): bool { return $this->__pos < count($this->__entries); }
    public function getMetadata(array $unserializeOptions = []): mixed { return null; }
    public function hasMetadata(): bool { return false; }
    public function setMetadata(mixed $metadata): void {}
    public function getSignature(): array|false { return false; }
    public function getStub(): string { return ''; }
    public function getVersion(): string { return '1.1.1'; }
    public function getAlias(): ?string { return null; }
    public function startBuffering(): void {}
    public function stopBuffering(): void {}
    public function isBuffering(): bool { return false; }
    final public static function canCompress(int $compression = 0): bool { return Phar::canCompress($compression); }
    final public static function canWrite(): bool { return true; }
    final public static function isValidPharFilename(string $filename, bool $executable = true): bool { return true; }
}
