
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
    public string $nodeName { get => __dom_prop(0, $this, 'nodeName'); set(mixed $value) { __dom_prop(1, $this, 'nodeName', $value); } }
    public ?string $nodeValue { get => __dom_prop(0, $this, 'nodeValue'); set(mixed $value) { __dom_prop(1, $this, 'nodeValue', $value); } }
    public int $nodeType { get => __dom_prop(0, $this, 'nodeType'); set(mixed $value) { __dom_prop(1, $this, 'nodeType', $value); } }
    public ?DOMNode $parentNode { get => __dom_prop(0, $this, 'parentNode'); set(mixed $value) { __dom_prop(1, $this, 'parentNode', $value); } }
    public ?DOMElement $parentElement { get => __dom_prop(0, $this, 'parentElement'); set(mixed $value) { __dom_prop(1, $this, 'parentElement', $value); } }
    public DOMNodeList $childNodes { get => __dom_prop(0, $this, 'childNodes'); set(mixed $value) { __dom_prop(1, $this, 'childNodes', $value); } }
    public ?DOMNode $firstChild { get => __dom_prop(0, $this, 'firstChild'); set(mixed $value) { __dom_prop(1, $this, 'firstChild', $value); } }
    public ?DOMNode $lastChild { get => __dom_prop(0, $this, 'lastChild'); set(mixed $value) { __dom_prop(1, $this, 'lastChild', $value); } }
    public ?DOMNode $previousSibling { get => __dom_prop(0, $this, 'previousSibling'); set(mixed $value) { __dom_prop(1, $this, 'previousSibling', $value); } }
    public ?DOMNode $nextSibling { get => __dom_prop(0, $this, 'nextSibling'); set(mixed $value) { __dom_prop(1, $this, 'nextSibling', $value); } }
    public ?DOMNamedNodeMap $attributes { get => __dom_prop(0, $this, 'attributes'); set(mixed $value) { __dom_prop(1, $this, 'attributes', $value); } }
    public bool $isConnected { get => __dom_prop(0, $this, 'isConnected'); set(mixed $value) { __dom_prop(1, $this, 'isConnected', $value); } }
    public ?DOMDocument $ownerDocument { get => __dom_prop(0, $this, 'ownerDocument'); set(mixed $value) { __dom_prop(1, $this, 'ownerDocument', $value); } }
    public ?string $namespaceURI { get => __dom_prop(0, $this, 'namespaceURI'); set(mixed $value) { __dom_prop(1, $this, 'namespaceURI', $value); } }
    public string $prefix { get => __dom_prop(0, $this, 'prefix'); set(mixed $value) { __dom_prop(1, $this, 'prefix', $value); } }
    public ?string $localName { get => __dom_prop(0, $this, 'localName'); set(mixed $value) { __dom_prop(1, $this, 'localName', $value); } }
    public ?string $baseURI { get => __dom_prop(0, $this, 'baseURI'); set(mixed $value) { __dom_prop(1, $this, 'baseURI', $value); } }
    public string $textContent { get => __dom_prop(0, $this, 'textContent'); set(mixed $value) { __dom_prop(1, $this, 'textContent', $value); } }
    public const DOCUMENT_POSITION_DISCONNECTED = 1;
    public const DOCUMENT_POSITION_PRECEDING = 2;
    public const DOCUMENT_POSITION_FOLLOWING = 4;
    public const DOCUMENT_POSITION_CONTAINS = 8;
    public const DOCUMENT_POSITION_CONTAINED_BY = 16;
    public const DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC = 32;

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
    public string $nodeName { get => __dom_prop(0, $this, 'nodeName'); set(mixed $value) { __dom_prop(1, $this, 'nodeName', $value); } }
    public ?string $nodeValue { get => __dom_prop(0, $this, 'nodeValue'); set(mixed $value) { __dom_prop(1, $this, 'nodeValue', $value); } }
    public int $nodeType { get => __dom_prop(0, $this, 'nodeType'); set(mixed $value) { __dom_prop(1, $this, 'nodeType', $value); } }
    public string $prefix { get => __dom_prop(0, $this, 'prefix'); set(mixed $value) { __dom_prop(1, $this, 'prefix', $value); } }
    public ?string $localName { get => __dom_prop(0, $this, 'localName'); set(mixed $value) { __dom_prop(1, $this, 'localName', $value); } }
    public ?string $namespaceURI { get => __dom_prop(0, $this, 'namespaceURI'); set(mixed $value) { __dom_prop(1, $this, 'namespaceURI', $value); } }
    public bool $isConnected { get => __dom_prop(0, $this, 'isConnected'); set(mixed $value) { __dom_prop(1, $this, 'isConnected', $value); } }
    public ?DOMDocument $ownerDocument { get => __dom_prop(0, $this, 'ownerDocument'); set(mixed $value) { __dom_prop(1, $this, 'ownerDocument', $value); } }
    public ?DOMNode $parentNode { get => __dom_prop(0, $this, 'parentNode'); set(mixed $value) { __dom_prop(1, $this, 'parentNode', $value); } }
    public ?DOMElement $parentElement { get => __dom_prop(0, $this, 'parentElement'); set(mixed $value) { __dom_prop(1, $this, 'parentElement', $value); } }
    public function __sleep(): array { return __dom('DOMNode::__sleep', $this); }
    public function __wakeup(): void { __dom('DOMNode::__wakeup', $this); }
}

class DOMImplementation {
    public function hasFeature(string $feature, string $version) { return __dom('DOMImplementation::hasFeature', $this, $feature, $version); }
    public function createDocumentType(string $qualifiedName, string $publicId = "", string $systemId = "") { return __dom('DOMImplementation::createDocumentType', $this, $qualifiedName, $publicId, $systemId); }
    public function createDocument(?string $namespace = null, string $qualifiedName = "", ?DOMDocumentType $doctype = null) { return __dom('DOMImplementation::createDocument', $this, $namespace, $qualifiedName, $doctype); }
}

class DOMDocumentFragment extends DOMNode implements DOMParentNode {
    public ?DOMElement $firstElementChild { get => __dom_prop(0, $this, 'firstElementChild'); set(mixed $value) { __dom_prop(1, $this, 'firstElementChild', $value); } }
    public ?DOMElement $lastElementChild { get => __dom_prop(0, $this, 'lastElementChild'); set(mixed $value) { __dom_prop(1, $this, 'lastElementChild', $value); } }
    public int $childElementCount { get => __dom_prop(0, $this, 'childElementCount'); set(mixed $value) { __dom_prop(1, $this, 'childElementCount', $value); } }
    public function __construct() { __dom('DOMDocumentFragment::__construct', $this); }
    public function appendXML(string $data) { return __dom('DOMDocumentFragment::appendXML', $this, $data); }
    public function append(...$nodes): void { __dom('DOMDocumentFragment::append', $this, ...$nodes); }
    public function prepend(...$nodes): void { __dom('DOMDocumentFragment::prepend', $this, ...$nodes); }
    public function replaceChildren(...$nodes): void { __dom('DOMDocumentFragment::replaceChildren', $this, ...$nodes); }
}

class DOMNodeList implements IteratorAggregate, Countable, ArrayAccess {
    public int $length { get => __dom_prop(0, $this, 'length'); set(mixed $value) { __dom_prop(1, $this, 'length', $value); } }
    public function count(): int { return __dom('DOMNodeList::count', $this); }
    public function getIterator(): Iterator { $i = 0; $c = null; while (($p = __dom_iter($this, $c, $i)) !== null) { $c = $p[1]; yield $p[0] => $c; $i++; } }
    public function item(int $index) { return __dom('DOMNodeList::item', $this, $index); }
    public function offsetGet($offset): mixed { return __dom('DOMNodeList::offsetGet', $this, $offset); }
    public function offsetExists($offset): bool { return __dom('DOMNodeList::offsetExists', $this, $offset); }
    public function offsetSet($offset, $value): void { throw new Error('Cannot use object of type ' . static::class . ' as array'); }
    public function offsetUnset($offset): void { throw new Error('Cannot use object of type ' . static::class . ' as array'); }
}

class DOMNamedNodeMap implements IteratorAggregate, Countable, ArrayAccess {
    public int $length { get => __dom_prop(0, $this, 'length'); set(mixed $value) { __dom_prop(1, $this, 'length', $value); } }
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
    public string $data { get => __dom_prop(0, $this, 'data'); set(mixed $value) { __dom_prop(1, $this, 'data', $value); } }
    public int $length { get => __dom_prop(0, $this, 'length'); set(mixed $value) { __dom_prop(1, $this, 'length', $value); } }
    public ?DOMElement $previousElementSibling { get => __dom_prop(0, $this, 'previousElementSibling'); set(mixed $value) { __dom_prop(1, $this, 'previousElementSibling', $value); } }
    public ?DOMElement $nextElementSibling { get => __dom_prop(0, $this, 'nextElementSibling'); set(mixed $value) { __dom_prop(1, $this, 'nextElementSibling', $value); } }
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
    public string $wholeText { get => __dom_prop(0, $this, 'wholeText'); set(mixed $value) { __dom_prop(1, $this, 'wholeText', $value); } }
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
    public string $name { get => __dom_prop(0, $this, 'name'); set(mixed $value) { __dom_prop(1, $this, 'name', $value); } }
    public bool $specified { get => __dom_prop(0, $this, 'specified'); set(mixed $value) { __dom_prop(1, $this, 'specified', $value); } }
    public string $value { get => __dom_prop(0, $this, 'value'); set(mixed $value) { __dom_prop(1, $this, 'value', $value); } }
    public ?DOMElement $ownerElement { get => __dom_prop(0, $this, 'ownerElement'); set(mixed $value) { __dom_prop(1, $this, 'ownerElement', $value); } }
    public mixed $schemaTypeInfo { get => __dom_prop(0, $this, 'schemaTypeInfo'); set(mixed $value) { __dom_prop(1, $this, 'schemaTypeInfo', $value); } }
    public function __construct(string $name, string $value = "") { __dom('DOMAttr::__construct', $this, ...func_get_args()); }
    public function isId() { return __dom('DOMAttr::isId', $this); }
}

class DOMElement extends DOMNode implements DOMParentNode, DOMChildNode {
    public string $tagName { get => __dom_prop(0, $this, 'tagName'); set(mixed $value) { __dom_prop(1, $this, 'tagName', $value); } }
    public string $className { get => __dom_prop(0, $this, 'className'); set(mixed $value) { __dom_prop(1, $this, 'className', $value); } }
    public string $id { get => __dom_prop(0, $this, 'id'); set(mixed $value) { __dom_prop(1, $this, 'id', $value); } }
    public mixed $schemaTypeInfo { get => __dom_prop(0, $this, 'schemaTypeInfo'); set(mixed $value) { __dom_prop(1, $this, 'schemaTypeInfo', $value); } }
    public ?DOMElement $firstElementChild { get => __dom_prop(0, $this, 'firstElementChild'); set(mixed $value) { __dom_prop(1, $this, 'firstElementChild', $value); } }
    public ?DOMElement $lastElementChild { get => __dom_prop(0, $this, 'lastElementChild'); set(mixed $value) { __dom_prop(1, $this, 'lastElementChild', $value); } }
    public int $childElementCount { get => __dom_prop(0, $this, 'childElementCount'); set(mixed $value) { __dom_prop(1, $this, 'childElementCount', $value); } }
    public ?DOMElement $previousElementSibling { get => __dom_prop(0, $this, 'previousElementSibling'); set(mixed $value) { __dom_prop(1, $this, 'previousElementSibling', $value); } }
    public ?DOMElement $nextElementSibling { get => __dom_prop(0, $this, 'nextElementSibling'); set(mixed $value) { __dom_prop(1, $this, 'nextElementSibling', $value); } }
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
    public ?DOMDocumentType $doctype { get => __dom_prop(0, $this, 'doctype'); set(mixed $value) { __dom_prop(1, $this, 'doctype', $value); } }
    public DOMImplementation $implementation { get => __dom_prop(0, $this, 'implementation'); set(mixed $value) { __dom_prop(1, $this, 'implementation', $value); } }
    public ?DOMElement $documentElement { get => __dom_prop(0, $this, 'documentElement'); set(mixed $value) { __dom_prop(1, $this, 'documentElement', $value); } }
    public ?string $actualEncoding { get => __dom_prop(0, $this, 'actualEncoding'); set(mixed $value) { __dom_prop(1, $this, 'actualEncoding', $value); } }
    public ?string $encoding { get => __dom_prop(0, $this, 'encoding'); set(mixed $value) { __dom_prop(1, $this, 'encoding', $value); } }
    public ?string $xmlEncoding { get => __dom_prop(0, $this, 'xmlEncoding'); set(mixed $value) { __dom_prop(1, $this, 'xmlEncoding', $value); } }
    public bool $standalone { get => __dom_prop(0, $this, 'standalone'); set(mixed $value) { __dom_prop(1, $this, 'standalone', $value); } }
    public bool $xmlStandalone { get => __dom_prop(0, $this, 'xmlStandalone'); set(mixed $value) { __dom_prop(1, $this, 'xmlStandalone', $value); } }
    public ?string $version { get => __dom_prop(0, $this, 'version'); set(mixed $value) { __dom_prop(1, $this, 'version', $value); } }
    public ?string $xmlVersion { get => __dom_prop(0, $this, 'xmlVersion'); set(mixed $value) { __dom_prop(1, $this, 'xmlVersion', $value); } }
    public bool $strictErrorChecking { get => __dom_prop(0, $this, 'strictErrorChecking'); set(mixed $value) { __dom_prop(1, $this, 'strictErrorChecking', $value); } }
    public ?string $documentURI { get => __dom_prop(0, $this, 'documentURI'); set(mixed $value) { __dom_prop(1, $this, 'documentURI', $value); } }
    public mixed $config { get => __dom_prop(0, $this, 'config'); set(mixed $value) { __dom_prop(1, $this, 'config', $value); } }
    public bool $formatOutput { get => __dom_prop(0, $this, 'formatOutput'); set(mixed $value) { __dom_prop(1, $this, 'formatOutput', $value); } }
    public bool $validateOnParse { get => __dom_prop(0, $this, 'validateOnParse'); set(mixed $value) { __dom_prop(1, $this, 'validateOnParse', $value); } }
    public bool $resolveExternals { get => __dom_prop(0, $this, 'resolveExternals'); set(mixed $value) { __dom_prop(1, $this, 'resolveExternals', $value); } }
    public bool $preserveWhiteSpace { get => __dom_prop(0, $this, 'preserveWhiteSpace'); set(mixed $value) { __dom_prop(1, $this, 'preserveWhiteSpace', $value); } }
    public bool $recover { get => __dom_prop(0, $this, 'recover'); set(mixed $value) { __dom_prop(1, $this, 'recover', $value); } }
    public bool $substituteEntities { get => __dom_prop(0, $this, 'substituteEntities'); set(mixed $value) { __dom_prop(1, $this, 'substituteEntities', $value); } }
    public ?DOMElement $firstElementChild { get => __dom_prop(0, $this, 'firstElementChild'); set(mixed $value) { __dom_prop(1, $this, 'firstElementChild', $value); } }
    public ?DOMElement $lastElementChild { get => __dom_prop(0, $this, 'lastElementChild'); set(mixed $value) { __dom_prop(1, $this, 'lastElementChild', $value); } }
    public int $childElementCount { get => __dom_prop(0, $this, 'childElementCount'); set(mixed $value) { __dom_prop(1, $this, 'childElementCount', $value); } }
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

class DOMDocumentType extends DOMNode {
    public string $name { get => __dom_prop(0, $this, 'name'); set(mixed $value) { __dom_prop(1, $this, 'name', $value); } }
    public DOMNamedNodeMap $entities { get => __dom_prop(0, $this, 'entities'); set(mixed $value) { __dom_prop(1, $this, 'entities', $value); } }
    public DOMNamedNodeMap $notations { get => __dom_prop(0, $this, 'notations'); set(mixed $value) { __dom_prop(1, $this, 'notations', $value); } }
    public string $publicId { get => __dom_prop(0, $this, 'publicId'); set(mixed $value) { __dom_prop(1, $this, 'publicId', $value); } }
    public string $systemId { get => __dom_prop(0, $this, 'systemId'); set(mixed $value) { __dom_prop(1, $this, 'systemId', $value); } }
    public ?string $internalSubset { get => __dom_prop(0, $this, 'internalSubset'); set(mixed $value) { __dom_prop(1, $this, 'internalSubset', $value); } }
}
class DOMEntity extends DOMNode {
    public ?string $publicId { get => __dom_prop(0, $this, 'publicId'); set(mixed $value) { __dom_prop(1, $this, 'publicId', $value); } }
    public ?string $systemId { get => __dom_prop(0, $this, 'systemId'); set(mixed $value) { __dom_prop(1, $this, 'systemId', $value); } }
    public ?string $notationName { get => __dom_prop(0, $this, 'notationName'); set(mixed $value) { __dom_prop(1, $this, 'notationName', $value); } }
    public ?string $actualEncoding { get => __dom_prop(0, $this, 'actualEncoding'); set(mixed $value) { __dom_prop(1, $this, 'actualEncoding', $value); } }
    public ?string $encoding { get => __dom_prop(0, $this, 'encoding'); set(mixed $value) { __dom_prop(1, $this, 'encoding', $value); } }
    public ?string $version { get => __dom_prop(0, $this, 'version'); set(mixed $value) { __dom_prop(1, $this, 'version', $value); } }
}
class DOMNotation extends DOMNode {
    public string $publicId { get => __dom_prop(0, $this, 'publicId'); set(mixed $value) { __dom_prop(1, $this, 'publicId', $value); } }
    public string $systemId { get => __dom_prop(0, $this, 'systemId'); set(mixed $value) { __dom_prop(1, $this, 'systemId', $value); } }
}

class DOMEntityReference extends DOMNode {
    public function __construct(string $name) { __dom('DOMEntityReference::__construct', $this, $name); }
}

class DOMProcessingInstruction extends DOMNode {
    public string $target { get => __dom_prop(0, $this, 'target'); set(mixed $value) { __dom_prop(1, $this, 'target', $value); } }
    public string $data { get => __dom_prop(0, $this, 'data'); set(mixed $value) { __dom_prop(1, $this, 'data', $value); } }
    public function __construct(string $name, string $value = "") { __dom('DOMProcessingInstruction::__construct', $this, ...func_get_args()); }
}

class DOMXPath {
    public DOMDocument $document { get => __dom_prop(0, $this, 'document'); set(mixed $value) { __dom_prop(1, $this, 'document', $value); } }
    public bool $registerNodeNamespaces { get => __dom_prop(0, $this, 'registerNodeNamespaces'); set(mixed $value) { __dom_prop(1, $this, 'registerNodeNamespaces', $value); } }
    public function __construct(DOMDocument $document, bool $registerNodeNS = true) { __dom('DOMXPath::__construct', $this, $document, $registerNodeNS); }
    public function evaluate(string $expression, ?DOMNode $contextNode = null, bool $registerNodeNS = true) { return __dom('DOMXPath::evaluate', $this, ...func_get_args()); }
    public function query(string $expression, ?DOMNode $contextNode = null, bool $registerNodeNS = true) { return __dom('DOMXPath::query', $this, ...func_get_args()); }
    public function registerNamespace(string $prefix, string $namespace) { return __dom('DOMXPath::registerNamespace', $this, $prefix, $namespace); }
    public function registerPhpFunctions($restrict = null) { __dom('DOMXPath::registerPhpFunctions', $this, $restrict); }
    public function registerPhpFunctionNS(string $namespaceURI, string $name, $callable) { __dom('DOMXPath::registerPhpFunctionNS', $this, $namespaceURI, $name, $callable); }
    public static function quote(string $str) { return __dom('DOMXPath::quote', null, $str); }
}
