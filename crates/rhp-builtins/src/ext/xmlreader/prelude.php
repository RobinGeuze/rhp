
class XMLReader {
    const NONE = 0; const ELEMENT = 1; const ATTRIBUTE = 2; const TEXT = 3; const CDATA = 4;
    const ENTITY_REF = 5; const ENTITY = 6; const PI = 7; const COMMENT = 8; const DOC = 9;
    const DOC_TYPE = 10; const DOC_FRAGMENT = 11; const NOTATION = 12; const WHITESPACE = 13;
    const SIGNIFICANT_WHITESPACE = 14; const END_ELEMENT = 15; const END_ENTITY = 16;
    const XML_DECLARATION = 17; const LOADDTD = 1; const DEFAULTATTRS = 2; const VALIDATE = 3;
    const SUBST_ENTITIES = 4;
    public function __get($name) {
        $v = __xr_prop(0, $this, $name);
        if (is_array($v)) { return $this->$name; }
        return $v;
    }
    public function __set($name, $value) {
        if (is_array(__xr_prop(1, $this, $name, $value))) { $this->$name = $value; }
    }
    public function __isset($name) {
        $v = __xr_prop(2, $this, $name);
        if (is_array($v)) { return isset($this->$name); }
        return $v;
    }
    public function __unset($name) {
        if (is_array(__xr_prop(4, $this, $name))) { unset($this->$name); }
    }
    public function close() { return __xr('XMLReader::close', $this); }
    public function getAttribute(string $name) { return __xr('XMLReader::getAttribute', $this, $name); }
    public function getAttributeNo(int $index) { return __xr('XMLReader::getAttributeNo', $this, $index); }
    public function getAttributeNs(string $name, string $namespace) { return __xr('XMLReader::getAttributeNs', $this, $name, $namespace); }
    public function getParserProperty(int $property) { return __xr('XMLReader::getParserProperty', $this, $property); }
    public function isValid() { return __xr('XMLReader::isValid', $this); }
    public function lookupNamespace(string $prefix) { return __xr('XMLReader::lookupNamespace', $this, $prefix); }
    public function moveToAttribute(string $name) { return __xr('XMLReader::moveToAttribute', $this, $name); }
    public function moveToAttributeNo(int $index) { return __xr('XMLReader::moveToAttributeNo', $this, $index); }
    public function moveToAttributeNs(string $name, string $namespace) { return __xr('XMLReader::moveToAttributeNs', $this, $name, $namespace); }
    public function moveToElement() { return __xr('XMLReader::moveToElement', $this); }
    public function moveToFirstAttribute() { return __xr('XMLReader::moveToFirstAttribute', $this); }
    public function moveToNextAttribute() { return __xr('XMLReader::moveToNextAttribute', $this); }
    public function read() { return __xr('XMLReader::read', $this); }
    public function next(?string $name = null) { return __xr('XMLReader::next', $this, $name); }
    public function readInnerXml() { return __xr('XMLReader::readInnerXml', $this); }
    public function readOuterXml() { return __xr('XMLReader::readOuterXml', $this); }
    public function readString() { return __xr('XMLReader::readString', $this); }
    public function setSchema(?string $filename) { return __xr('XMLReader::setSchema', $this, $filename); }
    public function setParserProperty(int $property, bool $value) { return __xr('XMLReader::setParserProperty', $this, $property, $value); }
    public function setRelaxNGSchema(?string $filename) { return __xr('XMLReader::setRelaxNGSchema', $this, $filename); }
    public function setRelaxNGSchemaSource(?string $source) { return __xr('XMLReader::setRelaxNGSchemaSource', $this, $source); }
    public function expand(?DOMNode $baseNode = null) { return __xr('XMLReader::expand', $this, $baseNode); }
    // open() and XML() are static methods that load into the instance when
    // called on one (php's get_method handler): the magic call hooks.
    // The source string is passed on as is: a cast would copy it.
    public function __call($name, $args) {
        $l = strtolower($name);
        if ($l !== 'open' && $l !== 'xml') { throw new Error('Call to undefined method ' . static::class . '::' . $name . '()'); }
        $s = $args[0] ?? '';
        if (!is_string($s)) { $s = (string) $s; }
        return __xr($l === 'open' ? 'XMLReader::open' : 'XMLReader::XML', $this, $s, $args[1] ?? null, (int) ($args[2] ?? 0));
    }
    public static function __callStatic($name, $args) {
        $l = strtolower($name);
        if ($l !== 'open' && $l !== 'xml') { throw new Error('Call to undefined method ' . static::class . '::' . $name . '()'); }
        $s = $args[0] ?? '';
        if (!is_string($s)) { $s = (string) $s; }
        return __xr($l === 'open' ? 'XMLReader::open' : 'XMLReader::XML', null, $s, $args[1] ?? null, (int) ($args[2] ?? 0));
    }
    public static function fromString(string $source, ?string $encoding = null, int $flags = 0) {
        return __xr('XMLReader::fromString', null, $source, $encoding, $flags, static::class);
    }
    public static function fromStream($stream, ?string $encoding = null, int $flags = 0, ?string $documentUri = null) {
        return __xr('XMLReader::fromStream', null, $stream, $encoding, $flags, $documentUri, static::class);
    }
    public static function fromUri(string $uri, ?string $encoding = null, int $flags = 0) {
        return __xr('XMLReader::fromUri', null, $uri, $encoding, $flags, static::class);
    }
}
