class ZipArchive implements Countable
{
    // %CONSTANTS%
    public function __get($name) { return __zip_prop(0, $this, $name) ?? $this->$name; }
    public function __set($name, $value) { __zip_prop(1, $this, $name); $this->$name = $value; }
    public function __isset($name) { return __zip_prop(2, $this, $name) ?? isset($this->$name); }
    public function __unset($name) { if (__zip_prop(3, $this, $name) === null) { unset($this->$name); } }

    public function open(string $filename, int $flags = 0): bool|int { return __zip('open', $this, $filename, $flags); }
    public function setPassword(#[\SensitiveParameter] string $password): bool { return __zip('setPassword', $this, $password); }
    public function close(): bool { return __zip('close', $this); }
    public function count(): int { return __zip('count', $this); }
    public function getStatusString(): string { return __zip('getStatusString', $this); }
    public function clearError(): void { __zip('clearError', $this); }
    public function addEmptyDir(string $dirname, int $flags = 0): bool { return __zip('addEmptyDir', $this, $dirname, $flags); }
    public function addFromString(string $name, string $content, int $flags = ZipArchive::FL_OVERWRITE): bool { return __zip('addFromString', $this, $name, $content, $flags); }
    public function addFile(string $filepath, string $entryname = "", int $start = 0, int $length = ZipArchive::LENGTH_TO_END, int $flags = ZipArchive::FL_OVERWRITE): bool { return __zip('addFile', $this, $filepath, $entryname, $start, $length, $flags); }
    public function replaceFile(string $filepath, int $index, int $start = 0, int $length = ZipArchive::LENGTH_TO_END, int $flags = 0): bool { return __zip('replaceFile', $this, $filepath, $index, $start, $length, $flags); }
    public function addGlob(string $pattern, int $flags = 0, array $options = []): array|false { return __zip('addGlob', $this, $pattern, $flags, $options); }
    public function addPattern(string $pattern, string $path = ".", array $options = []): array|false { return __zip('addPattern', $this, $pattern, $path, $options); }
    public function renameIndex(int $index, string $new_name): bool { return __zip('renameIndex', $this, $index, $new_name); }
    public function renameName(string $name, string $new_name): bool { return __zip('renameName', $this, $name, $new_name); }
    public function setArchiveComment(string $comment): bool { return __zip('setArchiveComment', $this, $comment); }
    public function getArchiveComment(int $flags = 0): string|false { return __zip('getArchiveComment', $this, $flags); }
    public function setArchiveFlag(int $flag, int $value): bool { return __zip('setArchiveFlag', $this, $flag, $value); }
    public function getArchiveFlag(int $flag, int $flags = 0): int { return __zip('getArchiveFlag', $this, $flag, $flags); }
    public function setCommentIndex(int $index, string $comment): bool { return __zip('setCommentIndex', $this, $index, $comment); }
    public function setCommentName(string $name, string $comment): bool { return __zip('setCommentName', $this, $name, $comment); }
    public function setMtimeIndex(int $index, int $timestamp, int $flags = 0): bool { return __zip('setMtimeIndex', $this, $index, $timestamp, $flags); }
    public function setMtimeName(string $name, int $timestamp, int $flags = 0): bool { return __zip('setMtimeName', $this, $name, $timestamp, $flags); }
    public function getCommentIndex(int $index, int $flags = 0): string|false { return __zip('getCommentIndex', $this, $index, $flags); }
    public function getCommentName(string $name, int $flags = 0): string|false { return __zip('getCommentName', $this, $name, $flags); }
    public function deleteIndex(int $index): bool { return __zip('deleteIndex', $this, $index); }
    public function deleteName(string $name): bool { return __zip('deleteName', $this, $name); }
    public function statName(string $name, int $flags = 0): array|false { return __zip('statName', $this, $name, $flags); }
    public function statIndex(int $index, int $flags = 0): array|false { return __zip('statIndex', $this, $index, $flags); }
    public function locateName(string $name, int $flags = 0): int|false { return __zip('locateName', $this, $name, $flags); }
    public function getNameIndex(int $index, int $flags = 0): string|false { return __zip('getNameIndex', $this, $index, $flags); }
    public function unchangeArchive(): bool { return __zip('unchangeArchive', $this); }
    public function unchangeAll(): bool { return __zip('unchangeAll', $this); }
    public function unchangeIndex(int $index): bool { return __zip('unchangeIndex', $this, $index); }
    public function unchangeName(string $name): bool { return __zip('unchangeName', $this, $name); }
    public function extractTo(string $pathto, array|string|null $files = null): bool { return __zip('extractTo', $this, $pathto, $files); }
    public function getFromName(string $name, int $len = 0, int $flags = 0): string|false { return __zip('getFromName', $this, $name, $len, $flags); }
    public function getFromIndex(int $index, int $len = 0, int $flags = 0): string|false { return __zip('getFromIndex', $this, $index, $len, $flags); }
    public function getStreamIndex(int $index, int $flags = 0) { return __zip('getStreamIndex', $this, $index, $flags); }
    public function getStreamName(string $name, int $flags = 0) { return __zip('getStreamName', $this, $name, $flags); }
    public function getStream(string $name) { return __zip('getStream', $this, $name); }
    public function setExternalAttributesName(string $name, int $opsys, int $attr, int $flags = 0): bool { return __zip('setExternalAttributesName', $this, $name, $opsys, $attr, $flags); }
    public function setExternalAttributesIndex(int $index, int $opsys, int $attr, int $flags = 0): bool { return __zip('setExternalAttributesIndex', $this, $index, $opsys, $attr, $flags); }
    public function getExternalAttributesName(string $name, &$opsys, &$attr, int $flags = 0): bool {
        $r = __zip('getExternalAttributesName', $this, $name, $flags);
        if ($r === false) { return false; }
        [$opsys, $attr] = $r;
        return true;
    }
    public function getExternalAttributesIndex(int $index, &$opsys, &$attr, int $flags = 0): bool {
        $r = __zip('getExternalAttributesIndex', $this, $index, $flags);
        if ($r === false) { return false; }
        [$opsys, $attr] = $r;
        return true;
    }
    public function setCompressionName(string $name, int $method, int $compflags = 0): bool { return __zip('setCompressionName', $this, $name, $method, $compflags); }
    public function setCompressionIndex(int $index, int $method, int $compflags = 0): bool { return __zip('setCompressionIndex', $this, $index, $method, $compflags); }
    public function setEncryptionName(string $name, int $method, #[\SensitiveParameter] ?string $password = null): bool { return __zip('setEncryptionName', $this, $name, $method, $password); }
    public function setEncryptionIndex(int $index, int $method, #[\SensitiveParameter] ?string $password = null): bool { return __zip('setEncryptionIndex', $this, $index, $method, $password); }
    public function registerProgressCallback(float $rate, $callback): bool { return __zip('registerProgressCallback', $this, $rate, $callback); }
    public function registerCancelCallback($callback): bool { return __zip('registerCancelCallback', $this, $callback); }
    public static function isCompressionMethodSupported(int $method, bool $enc = true): bool { return __zip('isCompressionMethodSupported', null, $method, $enc); }
    public static function isEncryptionMethodSupported(int $method, bool $enc = true): bool { return __zip('isEncryptionMethodSupported', null, $method, $enc); }
}
