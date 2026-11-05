
class SimpleXMLElement implements Stringable, Countable, RecursiveIterator, ArrayAccess {
    public function __construct(string $data, int $options = 0, bool $dataIsURL = false, string $namespaceOrPrefix = "", bool $isPrefix = false) { __sxe('SimpleXMLElement::__construct', $this, $data, $options, $dataIsURL, $namespaceOrPrefix, $isPrefix); }
    public function xpath(string $expression) { return __sxe('SimpleXMLElement::xpath', $this, $expression); }
    public function registerXPathNamespace(string $prefix, string $namespace) { return __sxe('SimpleXMLElement::registerXPathNamespace', $this, $prefix, $namespace); }
    public function asXML(?string $filename = null) { return __sxe('SimpleXMLElement::asXML', $this, $filename); }
    public function saveXML(?string $filename = null) { return __sxe('SimpleXMLElement::saveXML', $this, $filename); }
    public function getNamespaces(bool $recursive = false) { return __sxe('SimpleXMLElement::getNamespaces', $this, $recursive); }
    public function getDocNamespaces(bool $recursive = false, bool $fromRoot = true) { return __sxe('SimpleXMLElement::getDocNamespaces', $this, $recursive, $fromRoot); }
    public function children(?string $namespaceOrPrefix = null, bool $isPrefix = false) { return __sxe('SimpleXMLElement::children', $this, $namespaceOrPrefix, $isPrefix); }
    public function attributes(?string $namespaceOrPrefix = null, bool $isPrefix = false) { return __sxe('SimpleXMLElement::attributes', $this, $namespaceOrPrefix, $isPrefix); }
    public function addChild(string $qualifiedName, ?string $value = null, ?string $namespace = null) { return __sxe('SimpleXMLElement::addChild', $this, $qualifiedName, $value, $namespace); }
    public function addAttribute(string $qualifiedName, string $value, ?string $namespace = null) { __sxe('SimpleXMLElement::addAttribute', $this, $qualifiedName, $value, $namespace); }
    public function getName() { return __sxe('SimpleXMLElement::getName', $this); }
    public function __toString(): string { return __sxe('SimpleXMLElement::__toString', $this); }
    public function __debugInfo() { return __sxe('SimpleXMLElement::__debugInfo', $this); }
    public function count() { return __sxe('SimpleXMLElement::count', $this); }
    public function rewind() { __sxe('SimpleXMLElement::rewind', $this); }
    public function valid() { return __sxe('SimpleXMLElement::valid', $this); }
    public function current() { return __sxe('SimpleXMLElement::current', $this); }
    public function key() { return __sxe('SimpleXMLElement::key', $this); }
    public function next() { __sxe('SimpleXMLElement::next', $this); }
    public function hasChildren() { return __sxe('SimpleXMLElement::hasChildren', $this); }
    public function getChildren() { return __sxe('SimpleXMLElement::getChildren', $this); }

    public function __get($name) { return __sxe_prop(0, $this, $name); }
    public function __set($name, $value) { __sxe_prop(1, $this, $name, $value); }
    public function __isset($name) { return __sxe_prop(2, $this, $name); }
    public function __unset($name) { __sxe_prop(3, $this, $name); }
    public function offsetGet($offset) { return __sxe_dim(0, $this, $offset); }
    public function offsetSet($offset, $value) { __sxe_dim(1, $this, $offset, $value); }
    public function offsetExists($offset) { return __sxe_dim(2, $this, $offset); }
    public function offsetUnset($offset) { __sxe_dim(3, $this, $offset); }
}

class SimpleXMLIterator extends SimpleXMLElement {}
