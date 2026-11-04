
final class DOMException extends Exception {
    public $code = 0;
}

interface DOMParentNode {
    public function append(...$nodes): void;
    public function prepend(...$nodes): void;
    public function replaceChildren(...$nodes): void;
}

interface DOMChildNode {
    public function remove(): void;
    public function before(...$nodes): void;
    public function after(...$nodes): void;
    public function replaceWith(...$nodes): void;
}

class DOMNode {
    public const DOCUMENT_POSITION_DISCONNECTED = 1;
    public const DOCUMENT_POSITION_PRECEDING = 2;
    public const DOCUMENT_POSITION_FOLLOWING = 4;
    public const DOCUMENT_POSITION_CONTAINS = 8;
    public const DOCUMENT_POSITION_CONTAINED_BY = 16;
    public const DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC = 32;

    public function __get($name) {
        $v = __dom_prop(0, $this, $name);
        if (is_array($v)) { return $this->$name; }
        return $v;
    }
    public function __set($name, $value) {
        if (is_array(__dom_prop(1, $this, $name, $value))) { $this->$name = $value; }
    }
    public function __isset($name) {
        $v = __dom_prop(2, $this, $name);
        if (is_array($v)) { return isset($this->$name); }
        return $v;
    }
    public function __unset($name) {
        if (is_array(__dom_prop(3, $this, $name))) { unset($this->$name); }
    }

    public function appendChild(DOMNode $node) { return __dom('DOMNode::appendChild', $this, $node); }
    public function C14N(bool $exclusive = false, bool $withComments = false, ?array $xpath = null, ?array $nsPrefixes = null) { return __dom('DOMNode::C14N', $this, $exclusive, $withComments, $xpath, $nsPrefixes); }
    public function C14NFile(string $uri, bool $exclusive = false, bool $withComments = false, ?array $xpath = null, ?array $nsPrefixes = null) { return __dom('DOMNode::C14NFile', $this, $uri, $exclusive, $withComments, $xpath, $nsPrefixes); }
    public function cloneNode(bool $deep = false) { return __dom('DOMNode::cloneNode', $this, $deep); }
    public function getLineNo() { return __dom('DOMNode::getLineNo', $this); }
    public function getNodePath() { return __dom('DOMNode::getNodePath', $this); }
    public function hasAttributes() { return __dom('DOMNode::hasAttributes', $this); }
    public function hasChildNodes() { return __dom('DOMNode::hasChildNodes', $this); }
    public function insertBefore(DOMNode $node, ?DOMNode $child = null) { return __dom('DOMNode::insertBefore', $this, $node, $child); }
    public function isDefaultNamespace(string $namespace) { return __dom('DOMNode::isDefaultNamespace', $this, $namespace); }
    public function isSameNode(DOMNode $otherNode) { return __dom('DOMNode::isSameNode', $this, $otherNode); }
    public function isEqualNode(?DOMNode $otherNode) { return __dom('DOMNode::isEqualNode', $this, $otherNode); }
    public function isSupported(string $feature, string $version) { return __dom('DOMNode::isSupported', $this, $feature, $version); }
    public function lookupNamespaceURI(?string $prefix) { return __dom('DOMNode::lookupNamespaceURI', $this, $prefix); }
    public function lookupPrefix(string $namespace) { return __dom('DOMNode::lookupPrefix', $this, $namespace); }
    public function normalize() { __dom('DOMNode::normalize', $this); }
    public function removeChild(DOMNode $child) { return __dom('DOMNode::removeChild', $this, $child); }
    public function replaceChild(DOMNode $node, DOMNode $child) { return __dom('DOMNode::replaceChild', $this, $node, $child); }
    public function contains(DOMNode|DOMNameSpaceNode|null $other) { return __dom('DOMNode::contains', $this, $other); }
    public function getRootNode(?array $options = null) { return __dom('DOMNode::getRootNode', $this, $options); }
    public function compareDocumentPosition(DOMNode $other) { return __dom('DOMNode::compareDocumentPosition', $this, $other); }
    public function __sleep(): array { return __dom('DOMNode::__sleep', $this); }
    public function __wakeup(): void { __dom('DOMNode::__wakeup', $this); }
}

class DOMNameSpaceNode {
    public function __get($name) {
        $v = __dom_prop(0, $this, $name);
        if (is_array($v)) { return $this->$name; }
        return $v;
    }
    public function __set($name, $value) {
        if (is_array(__dom_prop(1, $this, $name, $value))) { $this->$name = $value; }
    }
    public function __isset($name) {
        $v = __dom_prop(2, $this, $name);
        if (is_array($v)) { return isset($this->$name); }
        return $v;
    }
    public function __unset($name) {
        if (is_array(__dom_prop(3, $this, $name))) { unset($this->$name); }
    }
    public function __sleep(): array { return __dom('DOMNode::__sleep', $this); }
    public function __wakeup(): void { __dom('DOMNode::__wakeup', $this); }
}

class DOMImplementation {
    public function hasFeature(string $feature, string $version) { return __dom('DOMImplementation::hasFeature', $this, $feature, $version); }
    public function createDocumentType(string $qualifiedName, string $publicId = "", string $systemId = "") { return __dom('DOMImplementation::createDocumentType', $this, $qualifiedName, $publicId, $systemId); }
    public function createDocument(?string $namespace = null, string $qualifiedName = "", ?DOMDocumentType $doctype = null) { return __dom('DOMImplementation::createDocument', $this, $namespace, $qualifiedName, $doctype); }
}

class DOMDocumentFragment extends DOMNode implements DOMParentNode {
    public function __construct() { __dom('DOMDocumentFragment::__construct', $this); }
    public function appendXML(string $data) { return __dom('DOMDocumentFragment::appendXML', $this, $data); }
    public function append(...$nodes): void { __dom('DOMDocumentFragment::append', $this, ...$nodes); }
    public function prepend(...$nodes): void { __dom('DOMDocumentFragment::prepend', $this, ...$nodes); }
    public function replaceChildren(...$nodes): void { __dom('DOMDocumentFragment::replaceChildren', $this, ...$nodes); }
}

class DOMNodeList implements IteratorAggregate, Countable, ArrayAccess {
    public function __get($name) {
        $v = __dom_prop(0, $this, $name);
        if (is_array($v)) { return $this->$name; }
        return $v;
    }
    public function __set($name, $value) {
        if (is_array(__dom_prop(1, $this, $name, $value))) { $this->$name = $value; }
    }
    public function __isset($name) {
        $v = __dom_prop(2, $this, $name);
        if (is_array($v)) { return isset($this->$name); }
        return $v;
    }
    public function __unset($name) {
        if (is_array(__dom_prop(3, $this, $name))) { unset($this->$name); }
    }
    public function count(): int { return __dom('DOMNodeList::count', $this); }
    public function getIterator(): Iterator { $i = 0; $c = null; while (($p = __dom_iter($this, $c, $i)) !== null) { $c = $p[1]; yield $p[0] => $c; $i++; } }
    public function item(int $index) { return __dom('DOMNodeList::item', $this, $index); }
    public function offsetGet($offset): mixed { return __dom('DOMNodeList::offsetGet', $this, $offset); }
    public function offsetExists($offset): bool { return __dom('DOMNodeList::offsetExists', $this, $offset); }
    public function offsetSet($offset, $value): void { throw new Error('Cannot use object of type ' . static::class . ' as array'); }
    public function offsetUnset($offset): void { throw new Error('Cannot use object of type ' . static::class . ' as array'); }
}

class DOMNamedNodeMap implements IteratorAggregate, Countable, ArrayAccess {
    public function __get($name) {
        $v = __dom_prop(0, $this, $name);
        if (is_array($v)) { return $this->$name; }
        return $v;
    }
    public function __set($name, $value) {
        if (is_array(__dom_prop(1, $this, $name, $value))) { $this->$name = $value; }
    }
    public function __isset($name) {
        $v = __dom_prop(2, $this, $name);
        if (is_array($v)) { return isset($this->$name); }
        return $v;
    }
    public function __unset($name) {
        if (is_array(__dom_prop(3, $this, $name))) { unset($this->$name); }
    }
    public function getNamedItem(string $qualifiedName): ?DOMNode { return __dom('DOMNamedNodeMap::getNamedItem', $this, $qualifiedName); }
    public function getNamedItemNS(?string $namespace, string $localName): ?DOMNode { return __dom('DOMNamedNodeMap::getNamedItemNS', $this, $namespace, $localName); }
    public function item(int $index): ?DOMNode { return __dom('DOMNamedNodeMap::item', $this, $index); }
    public function count(): int { return __dom('DOMNamedNodeMap::count', $this); }
    public function getIterator(): Iterator { $i = 0; $c = null; while (($p = __dom_iter($this, $c, $i)) !== null) { $c = $p[1]; yield $p[0] => $c; $i++; } }
    public function offsetGet($offset): mixed { return __dom('DOMNamedNodeMap::offsetGet', $this, $offset); }
    public function offsetExists($offset): bool { return __dom('DOMNamedNodeMap::offsetExists', $this, $offset); }
    public function offsetSet($offset, $value): void { throw new Error('Cannot use object of type ' . static::class . ' as array'); }
    public function offsetUnset($offset): void { throw new Error('Cannot use object of type ' . static::class . ' as array'); }
}

class DOMCharacterData extends DOMNode implements DOMChildNode {
    public function appendData(string $data) { return __dom('DOMCharacterData::appendData', $this, $data); }
    public function substringData(int $offset, int $count) { return __dom('DOMCharacterData::substringData', $this, $offset, $count); }
    public function insertData(int $offset, string $data) { return __dom('DOMCharacterData::insertData', $this, $offset, $data); }
    public function deleteData(int $offset, int $count) { return __dom('DOMCharacterData::deleteData', $this, $offset, $count); }
    public function replaceData(int $offset, int $count, string $data) { return __dom('DOMCharacterData::replaceData', $this, $offset, $count, $data); }
    public function replaceWith(...$nodes): void { __dom('DOMCharacterData::replaceWith', $this, ...$nodes); }
    public function remove(): void { __dom('DOMCharacterData::remove', $this); }
    public function before(...$nodes): void { __dom('DOMCharacterData::before', $this, ...$nodes); }
    public function after(...$nodes): void { __dom('DOMCharacterData::after', $this, ...$nodes); }
}

class DOMText extends DOMCharacterData {
    public function __construct(string $data = "") { __dom('DOMText::__construct', $this, ...func_get_args()); }
    public function isWhitespaceInElementContent() { return __dom('DOMText::isWhitespaceInElementContent', $this); }
    public function isElementContentWhitespace() { return __dom('DOMText::isElementContentWhitespace', $this); }
    public function splitText(int $offset) { return __dom('DOMText::splitText', $this, $offset); }
}

class DOMCdataSection extends DOMText {
    public function __construct(string $data) { __dom('DOMCdataSection::__construct', $this, $data); }
}

class DOMComment extends DOMCharacterData {
    public function __construct(string $data = "") { __dom('DOMComment::__construct', $this, ...func_get_args()); }
}

class DOMAttr extends DOMNode {
    public function __construct(string $name, string $value = "") { __dom('DOMAttr::__construct', $this, ...func_get_args()); }
    public function isId() { return __dom('DOMAttr::isId', $this); }
}

class DOMElement extends DOMNode implements DOMParentNode, DOMChildNode {
    public function __construct(string $qualifiedName, ?string $value = null, string $namespace = "") { __dom('DOMElement::__construct', $this, $qualifiedName, $value, $namespace); }
    public function getAttribute(string $qualifiedName) { return __dom('DOMElement::getAttribute', $this, $qualifiedName); }
    public function getAttributeNames() { return __dom('DOMElement::getAttributeNames', $this); }
    public function getAttributeNS(?string $namespace, string $localName) { return __dom('DOMElement::getAttributeNS', $this, $namespace, $localName); }
    public function getAttributeNode(string $qualifiedName) { return __dom('DOMElement::getAttributeNode', $this, $qualifiedName); }
    public function getAttributeNodeNS(?string $namespace, string $localName) { return __dom('DOMElement::getAttributeNodeNS', $this, $namespace, $localName); }
    public function getElementsByTagName(string $qualifiedName) { return __dom('DOMElement::getElementsByTagName', $this, $qualifiedName); }
    public function getElementsByTagNameNS(?string $namespace, string $localName) { return __dom('DOMElement::getElementsByTagNameNS', $this, $namespace, $localName); }
    public function hasAttribute(string $qualifiedName) { return __dom('DOMElement::hasAttribute', $this, $qualifiedName); }
    public function hasAttributeNS(?string $namespace, string $localName) { return __dom('DOMElement::hasAttributeNS', $this, $namespace, $localName); }
    public function removeAttribute(string $qualifiedName) { return __dom('DOMElement::removeAttribute', $this, $qualifiedName); }
    public function removeAttributeNS(?string $namespace, string $localName) { __dom('DOMElement::removeAttributeNS', $this, $namespace, $localName); }
    public function removeAttributeNode(DOMAttr $attr) { return __dom('DOMElement::removeAttributeNode', $this, $attr); }
    public function setAttribute(string $qualifiedName, string $value) { return __dom('DOMElement::setAttribute', $this, $qualifiedName, $value); }
    public function setAttributeNS(?string $namespace, string $qualifiedName, string $value) { __dom('DOMElement::setAttributeNS', $this, $namespace, $qualifiedName, $value); }
    public function setAttributeNode(DOMAttr $attr) { return __dom('DOMElement::setAttributeNode', $this, $attr); }
    public function setAttributeNodeNS(DOMAttr $attr) { return __dom('DOMElement::setAttributeNodeNS', $this, $attr); }
    public function setIdAttribute(string $qualifiedName, bool $isId) { __dom('DOMElement::setIdAttribute', $this, $qualifiedName, $isId); }
    public function setIdAttributeNS(string $namespace, string $qualifiedName, bool $isId) { __dom('DOMElement::setIdAttributeNS', $this, $namespace, $qualifiedName, $isId); }
    public function setIdAttributeNode(DOMAttr $attr, bool $isId) { __dom('DOMElement::setIdAttributeNode', $this, $attr, $isId); }
    public function toggleAttribute(string $qualifiedName, ?bool $force = null) { return __dom('DOMElement::toggleAttribute', $this, $qualifiedName, $force); }
    public function remove(): void { __dom('DOMElement::remove', $this); }
    public function before(...$nodes): void { __dom('DOMElement::before', $this, ...$nodes); }
    public function after(...$nodes): void { __dom('DOMElement::after', $this, ...$nodes); }
    public function replaceWith(...$nodes): void { __dom('DOMElement::replaceWith', $this, ...$nodes); }
    public function append(...$nodes): void { __dom('DOMElement::append', $this, ...$nodes); }
    public function prepend(...$nodes): void { __dom('DOMElement::prepend', $this, ...$nodes); }
    public function replaceChildren(...$nodes): void { __dom('DOMElement::replaceChildren', $this, ...$nodes); }
    public function insertAdjacentElement(string $where, DOMElement $element) { return __dom('DOMElement::insertAdjacentElement', $this, $where, $element); }
    public function insertAdjacentText(string $where, string $data) { __dom('DOMElement::insertAdjacentText', $this, $where, $data); }
}

class DOMDocument extends DOMNode implements DOMParentNode {
    public function __construct(string $version = "1.0", string $encoding = "") { __dom('DOMDocument::__construct', $this, ...func_get_args()); }
    public function createAttribute(string $localName) { return __dom('DOMDocument::createAttribute', $this, $localName); }
    public function createAttributeNS(?string $namespace, string $qualifiedName) { return __dom('DOMDocument::createAttributeNS', $this, $namespace, $qualifiedName); }
    public function createCDATASection(string $data) { return __dom('DOMDocument::createCDATASection', $this, $data); }
    public function createComment(string $data) { return __dom('DOMDocument::createComment', $this, $data); }
    public function createDocumentFragment() { return __dom('DOMDocument::createDocumentFragment', $this); }
    public function createElement(string $localName, string $value = "") { return __dom('DOMDocument::createElement', $this, ...func_get_args()); }
    public function createElementNS(?string $namespace, string $qualifiedName, string $value = "") { return __dom('DOMDocument::createElementNS', $this, ...func_get_args()); }
    public function createEntityReference(string $name) { return __dom('DOMDocument::createEntityReference', $this, $name); }
    public function createProcessingInstruction(string $target, string $data = "") { return __dom('DOMDocument::createProcessingInstruction', $this, ...func_get_args()); }
    public function createTextNode(string $data) { return __dom('DOMDocument::createTextNode', $this, $data); }
    public function getElementById(string $elementId) { return __dom('DOMDocument::getElementById', $this, $elementId); }
    public function getElementsByTagName(string $qualifiedName) { return __dom('DOMDocument::getElementsByTagName', $this, $qualifiedName); }
    public function getElementsByTagNameNS(?string $namespace, string $localName) { return __dom('DOMDocument::getElementsByTagNameNS', $this, $namespace, $localName); }
    public function importNode(DOMNode $node, bool $deep = false) { return __dom('DOMDocument::importNode', $this, $node, $deep); }
    public function load(string $filename, int $options = 0) { return __dom('DOMDocument::load', $this, $filename, $options); }
    public function loadXML(string $source, int $options = 0) { return __dom('DOMDocument::loadXML', $this, $source, $options); }
    public function normalizeDocument() { __dom('DOMDocument::normalizeDocument', $this); }
    public function registerNodeClass(string $baseClass, ?string $extendedClass) { return __dom('DOMDocument::registerNodeClass', $this, $baseClass, $extendedClass); }
    public function save(string $filename, int $options = 0) { return __dom('DOMDocument::save', $this, $filename, $options); }
    public function loadHTML(string $source, int $options = 0) { return __dom('DOMDocument::loadHTML', $this, $source, $options); }
    public function loadHTMLFile(string $filename, int $options = 0) { return __dom('DOMDocument::loadHTMLFile', $this, $filename, $options); }
    public function saveHTML(?DOMNode $node = null) { return __dom('DOMDocument::saveHTML', $this, $node); }
    public function saveHTMLFile(string $filename) { return __dom('DOMDocument::saveHTMLFile', $this, $filename); }
    public function saveXML(?DOMNode $node = null, int $options = 0) { return __dom('DOMDocument::saveXML', $this, $node, $options); }
    public function schemaValidate(string $filename, int $flags = 0) { return __dom('DOMDocument::schemaValidate', $this, $filename, $flags); }
    public function schemaValidateSource(string $source, int $flags = 0) { return __dom('DOMDocument::schemaValidateSource', $this, $source, $flags); }
    public function relaxNGValidate(string $filename) { return __dom('DOMDocument::relaxNGValidate', $this, $filename); }
    public function relaxNGValidateSource(string $source) { return __dom('DOMDocument::relaxNGValidateSource', $this, $source); }
    public function validate() { return __dom('DOMDocument::validate', $this); }
    public function xinclude(int $options = 0) { return __dom('DOMDocument::xinclude', $this, $options); }
    public function adoptNode(DOMNode $node) { return __dom('DOMDocument::adoptNode', $this, $node); }
    public function append(...$nodes): void { __dom('DOMDocument::append', $this, ...$nodes); }
    public function prepend(...$nodes): void { __dom('DOMDocument::prepend', $this, ...$nodes); }
    public function replaceChildren(...$nodes): void { __dom('DOMDocument::replaceChildren', $this, ...$nodes); }
}

class DOMDocumentType extends DOMNode {}
class DOMEntity extends DOMNode {}
class DOMNotation extends DOMNode {}

class DOMEntityReference extends DOMNode {
    public function __construct(string $name) { __dom('DOMEntityReference::__construct', $this, $name); }
}

class DOMProcessingInstruction extends DOMNode {
    public function __construct(string $name, string $value = "") { __dom('DOMProcessingInstruction::__construct', $this, ...func_get_args()); }
}

class DOMXPath {
    public function __get($name) {
        $v = __dom_prop(0, $this, $name);
        if (is_array($v)) { return $this->$name; }
        return $v;
    }
    public function __set($name, $value) {
        if (is_array(__dom_prop(1, $this, $name, $value))) { $this->$name = $value; }
    }
    public function __isset($name) {
        $v = __dom_prop(2, $this, $name);
        if (is_array($v)) { return isset($this->$name); }
        return $v;
    }
    public function __unset($name) {
        if (is_array(__dom_prop(3, $this, $name))) { unset($this->$name); }
    }
    public function __construct(DOMDocument $document, bool $registerNodeNS = true) { __dom('DOMXPath::__construct', $this, $document, $registerNodeNS); }
    public function evaluate(string $expression, ?DOMNode $contextNode = null, bool $registerNodeNS = true) { return __dom('DOMXPath::evaluate', $this, ...func_get_args()); }
    public function query(string $expression, ?DOMNode $contextNode = null, bool $registerNodeNS = true) { return __dom('DOMXPath::query', $this, ...func_get_args()); }
    public function registerNamespace(string $prefix, string $namespace) { return __dom('DOMXPath::registerNamespace', $this, $prefix, $namespace); }
    public function registerPhpFunctions($restrict = null) { __dom('DOMXPath::registerPhpFunctions', $this, $restrict); }
    public function registerPhpFunctionNS(string $namespaceURI, string $name, $callable) { __dom('DOMXPath::registerPhpFunctionNS', $this, $namespaceURI, $name, $callable); }
    public static function quote(string $str) { return __dom('DOMXPath::quote', null, $str); }
}
