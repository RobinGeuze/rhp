
final class CurlHandle {
    public function __construct() { throw new Error("Cannot directly construct CurlHandle, use curl_init() instead"); }
}
final class CurlMultiHandle {
    public function __construct() { throw new Error("Cannot directly construct CurlMultiHandle, use curl_multi_init() instead"); }
}
final class CurlShareHandle {
    public function __construct() { throw new Error("Cannot directly construct CurlShareHandle, use curl_share_init() instead"); }
}
final class CurlSharePersistentHandle {
    public readonly array $options;
    public function __construct() { throw new Error("Cannot directly construct CurlSharePersistentHandle, use curl_share_init_persistent() instead"); }
}
class CURLFile {
    public string $name = "";
    public string $mime = "";
    public string $postname = "";
    public function __construct(string $filename, ?string $mime_type = null, ?string $posted_filename = null) {
        if (str_contains($filename, "\0")) {
            throw new ValueError('CURLFile::__construct(): Argument #1 ($filename) must not contain any null bytes');
        }
        $this->name = $filename;
        if ($mime_type !== null) { $this->mime = $mime_type; }
        if ($posted_filename !== null) { $this->postname = $posted_filename; }
    }
    public function getFilename(): string { return $this->name; }
    public function getMimeType(): string { return $this->mime; }
    public function getPostFilename(): string { return $this->postname; }
    public function setMimeType(string $mime_type): void { $this->mime = $mime_type; }
    public function setPostFilename(string $posted_filename): void { $this->postname = $posted_filename; }
}
class CURLStringFile {
    public string $data;
    public string $postname;
    public string $mime;
    public function __construct(string $data, string $postname, string $mime = "application/octet-stream") {
        $this->data = $data;
        $this->postname = $postname;
        $this->mime = $mime;
    }
}
