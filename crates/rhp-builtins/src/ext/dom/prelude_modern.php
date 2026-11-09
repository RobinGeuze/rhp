namespace Dom {
    interface ParentNode {
        public function append(\Dom\Node|string ...$nodes): void;
        public function prepend(\Dom\Node|string ...$nodes): void;
        public function replaceChildren(\Dom\Node|string ...$nodes): void;
        public function querySelector(string $selectors): ?\Dom\Element;
        public function querySelectorAll(string $selectors): \Dom\NodeList;
    }
    interface ChildNode {
        public function remove(): void;
        public function before(\Dom\Node|string ...$nodes): void;
        public function after(\Dom\Node|string ...$nodes): void;
        public function replaceWith(\Dom\Node|string ...$nodes): void;
    }
    enum AdjacentPosition: string {
        case BeforeBegin = 'beforebegin';
        case AfterBegin = 'afterbegin';
        case BeforeEnd = 'beforeend';
        case AfterEnd = 'afterend';
    }
    class Implementation {
        public function createDocumentType(string $qualifiedName, string $publicId, string $systemId): \Dom\DocumentType { return __dom('Dom\Implementation::createDocumentType', $this, $qualifiedName, $publicId, $systemId); }
        public function createDocument(?string $namespace, string $qualifiedName, ?\Dom\DocumentType $doctype = null): \Dom\XMLDocument { return __dom('Dom\Implementation::createDocument', $this, $namespace, $qualifiedName, $doctype); }
        public function createHTMLDocument(?string $title = null): \Dom\HTMLDocument { return __dom('Dom\Implementation::createHTMLDocument', $this, $title); }
    }
    class Node {
        public const int DOCUMENT_POSITION_DISCONNECTED = 1;
        public const int DOCUMENT_POSITION_PRECEDING = 2;
        public const int DOCUMENT_POSITION_FOLLOWING = 4;
        public const int DOCUMENT_POSITION_CONTAINS = 8;
        public const int DOCUMENT_POSITION_CONTAINED_BY = 16;
        public const int DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC = 32;
        public int $nodeType { get => __dom_prop(0, $this, 'nodeType'); set(mixed $value) { __dom_prop(1, $this, 'nodeType', $value); } }
        public string $nodeName { get => __dom_prop(0, $this, 'nodeName'); set(mixed $value) { __dom_prop(1, $this, 'nodeName', $value); } }
        public string $baseURI { get => __dom_prop(0, $this, 'baseURI'); set(mixed $value) { __dom_prop(1, $this, 'baseURI', $value); } }
        public bool $isConnected { get => __dom_prop(0, $this, 'isConnected'); set(mixed $value) { __dom_prop(1, $this, 'isConnected', $value); } }
        public ?\Dom\Document $ownerDocument { get => __dom_prop(0, $this, 'ownerDocument'); set(mixed $value) { __dom_prop(1, $this, 'ownerDocument', $value); } }
        public ?\Dom\Node $parentNode { get => __dom_prop(0, $this, 'parentNode'); set(mixed $value) { __dom_prop(1, $this, 'parentNode', $value); } }
        public ?\Dom\Element $parentElement { get => __dom_prop(0, $this, 'parentElement'); set(mixed $value) { __dom_prop(1, $this, 'parentElement', $value); } }
        public \Dom\NodeList $childNodes { get => __dom_prop(0, $this, 'childNodes'); set(mixed $value) { __dom_prop(1, $this, 'childNodes', $value); } }
        public ?\Dom\Node $firstChild { get => __dom_prop(0, $this, 'firstChild'); set(mixed $value) { __dom_prop(1, $this, 'firstChild', $value); } }
        public ?\Dom\Node $lastChild { get => __dom_prop(0, $this, 'lastChild'); set(mixed $value) { __dom_prop(1, $this, 'lastChild', $value); } }
        public ?\Dom\Node $previousSibling { get => __dom_prop(0, $this, 'previousSibling'); set(mixed $value) { __dom_prop(1, $this, 'previousSibling', $value); } }
        public ?\Dom\Node $nextSibling { get => __dom_prop(0, $this, 'nextSibling'); set(mixed $value) { __dom_prop(1, $this, 'nextSibling', $value); } }
        public ?string $nodeValue { get => __dom_prop(0, $this, 'nodeValue'); set(mixed $value) { __dom_prop(1, $this, 'nodeValue', $value); } }
        public ?string $textContent { get => __dom_prop(0, $this, 'textContent'); set(mixed $value) { __dom_prop(1, $this, 'textContent', $value); } }
        final private function __construct() { throw new \Error('Cannot directly construct ' . static::class . ', use document methods instead'); }
        public function getRootNode(array $options = []): \Dom\Node { return __dom('Dom\Node::getRootNode', $this, $options); }
        public function hasChildNodes(): bool { return __dom('Dom\Node::hasChildNodes', $this); }
        public function normalize(): void { __dom('Dom\Node::normalize', $this); }
        public function cloneNode(bool $deep = false): \Dom\Node { return __dom('Dom\Node::cloneNode', $this, $deep); }
        public function isEqualNode(?\Dom\Node $otherNode): bool { return __dom('Dom\Node::isEqualNode', $this, $otherNode); }
        public function isSameNode(?\Dom\Node $otherNode): bool { return __dom('Dom\Node::isSameNode', $this, $otherNode); }
        public function compareDocumentPosition(\Dom\Node $other): int { return __dom('Dom\Node::compareDocumentPosition', $this, $other); }
        public function contains(?\Dom\Node $other): bool { return __dom('Dom\Node::contains', $this, $other); }
        public function lookupPrefix(?string $namespace): ?string { return __dom('Dom\Node::lookupPrefix', $this, $namespace); }
        public function lookupNamespaceURI(?string $prefix): ?string { return __dom('Dom\Node::lookupNamespaceURI', $this, $prefix); }
        public function isDefaultNamespace(?string $namespace): bool { return __dom('Dom\Node::isDefaultNamespace', $this, $namespace); }
        public function insertBefore(\Dom\Node $node, ?\Dom\Node $child): \Dom\Node { return __dom('Dom\Node::insertBefore', $this, $node, $child); }
        public function appendChild(\Dom\Node $node): \Dom\Node { return __dom('Dom\Node::appendChild', $this, $node); }
        public function replaceChild(\Dom\Node $node, \Dom\Node $child): \Dom\Node { return __dom('Dom\Node::replaceChild', $this, $node, $child); }
        public function removeChild(\Dom\Node $child): \Dom\Node { return __dom('Dom\Node::removeChild', $this, $child); }
        public function getLineNo(): int { return __dom('Dom\Node::getLineNo', $this); }
        public function getNodePath(): string { return __dom('Dom\Node::getNodePath', $this); }
        public function C14N(bool $exclusive = false, bool $withComments = false, ?array $xpath = null, ?array $nsPrefixes = null): string|false { return __dom('Dom\Node::C14N', $this, $exclusive, $withComments, $xpath, $nsPrefixes); }
        public function C14NFile(string $uri, bool $exclusive = false, bool $withComments = false, ?array $xpath = null, ?array $nsPrefixes = null): int|false { return __dom('Dom\Node::C14NFile', $this, $uri, $exclusive, $withComments, $xpath, $nsPrefixes); }
        public function __sleep(): array { return __dom('Dom\Node::__sleep', $this); }
        public function __wakeup(): void { __dom('Dom\Node::__wakeup', $this); }
    }
    final readonly class NamespaceInfo {
        public ?string $prefix;
        public ?string $namespaceURI;
        public \Dom\Element $element;
        private function __construct() { throw new \Error('Cannot directly construct ' . static::class . ', use document methods instead'); }
    }
    class DocumentFragment extends \Dom\Node implements \Dom\ParentNode {
        public \Dom\HTMLCollection $children { get => $this->children ??= __dom_prop(0, $this, 'children'); set(mixed $value) { __dom_prop(1, $this, 'children', $value); } }
        public ?\Dom\Element $firstElementChild { get => __dom_prop(0, $this, 'firstElementChild'); set(mixed $value) { __dom_prop(1, $this, 'firstElementChild', $value); } }
        public ?\Dom\Element $lastElementChild { get => __dom_prop(0, $this, 'lastElementChild'); set(mixed $value) { __dom_prop(1, $this, 'lastElementChild', $value); } }
        public int $childElementCount { get => __dom_prop(0, $this, 'childElementCount'); set(mixed $value) { __dom_prop(1, $this, 'childElementCount', $value); } }
        public function appendXml(string $data): bool { return __dom('Dom\DocumentFragment::appendXml', $this, $data); }
        public function append(\Dom\Node|string ...$nodes): void { __dom('Dom\DocumentFragment::append', $this, ...$nodes); }
        public function prepend(\Dom\Node|string ...$nodes): void { __dom('Dom\DocumentFragment::prepend', $this, ...$nodes); }
        public function replaceChildren(\Dom\Node|string ...$nodes): void { __dom('Dom\DocumentFragment::replaceChildren', $this, ...$nodes); }
        public function querySelector(string $selectors): ?\Dom\Element { return __dom('Dom\DocumentFragment::querySelector', $this, $selectors); }
        public function querySelectorAll(string $selectors): \Dom\NodeList { return __dom('Dom\DocumentFragment::querySelectorAll', $this, $selectors); }
    }
    abstract class Document extends \Dom\Node implements \Dom\ParentNode {
        public \Dom\HTMLCollection $children { get => $this->children ??= __dom_prop(0, $this, 'children'); set(mixed $value) { __dom_prop(1, $this, 'children', $value); } }
        public ?\Dom\Element $firstElementChild { get => __dom_prop(0, $this, 'firstElementChild'); set(mixed $value) { __dom_prop(1, $this, 'firstElementChild', $value); } }
        public ?\Dom\Element $lastElementChild { get => __dom_prop(0, $this, 'lastElementChild'); set(mixed $value) { __dom_prop(1, $this, 'lastElementChild', $value); } }
        public int $childElementCount { get => __dom_prop(0, $this, 'childElementCount'); set(mixed $value) { __dom_prop(1, $this, 'childElementCount', $value); } }
        public \Dom\Implementation $implementation { get => $this->implementation ??= __dom_prop(0, $this, 'implementation'); set(mixed $value) { __dom_prop(1, $this, 'implementation', $value); } }
        public string $URL { get => __dom_prop(0, $this, 'URL'); set(mixed $value) { __dom_prop(1, $this, 'URL', $value); } }
        public string $documentURI { get => __dom_prop(0, $this, 'documentURI'); set(mixed $value) { __dom_prop(1, $this, 'documentURI', $value); } }
        public string $characterSet { get => __dom_prop(0, $this, 'characterSet'); set(mixed $value) { __dom_prop(1, $this, 'characterSet', $value); } }
        public string $charset { get => __dom_prop(0, $this, 'charset'); set(mixed $value) { __dom_prop(1, $this, 'charset', $value); } }
        public string $inputEncoding { get => __dom_prop(0, $this, 'inputEncoding'); set(mixed $value) { __dom_prop(1, $this, 'inputEncoding', $value); } }
        public ?\Dom\DocumentType $doctype { get => __dom_prop(0, $this, 'doctype'); set(mixed $value) { __dom_prop(1, $this, 'doctype', $value); } }
        public ?\Dom\Element $documentElement { get => __dom_prop(0, $this, 'documentElement'); set(mixed $value) { __dom_prop(1, $this, 'documentElement', $value); } }
        public ?\Dom\HTMLElement $body { get => __dom_prop(0, $this, 'body'); set(mixed $value) { __dom_prop(1, $this, 'body', $value); } }
        public ?\Dom\HTMLElement $head { get => __dom_prop(0, $this, 'head'); set(mixed $value) { __dom_prop(1, $this, 'head', $value); } }
        public string $title { get => __dom_prop(0, $this, 'title'); set(mixed $value) { __dom_prop(1, $this, 'title', $value); } }
        public function getElementsByTagName(string $qualifiedName): \Dom\HTMLCollection { return __dom('Dom\Document::getElementsByTagName', $this, $qualifiedName); }
        public function getElementsByTagNameNS(?string $namespace, string $localName): \Dom\HTMLCollection { return __dom('Dom\Document::getElementsByTagNameNS', $this, $namespace, $localName); }
        public function getElementsByClassName(string $classNames): \Dom\HTMLCollection { return __dom('Dom\Document::getElementsByClassName', $this, $classNames); }
        public function createElement(string $localName): \Dom\Element { return __dom('Dom\Document::createElement', $this, $localName); }
        public function createElementNS(?string $namespace, string $qualifiedName): \Dom\Element { return __dom('Dom\Document::createElementNS', $this, $namespace, $qualifiedName); }
        public function createDocumentFragment(): \Dom\DocumentFragment { return __dom('Dom\Document::createDocumentFragment', $this); }
        public function createTextNode(string $data): \Dom\Text { return __dom('Dom\Document::createTextNode', $this, $data); }
        public function createCDATASection(string $data): \Dom\CDATASection { return __dom('Dom\Document::createCDATASection', $this, $data); }
        public function createComment(string $data): \Dom\Comment { return __dom('Dom\Document::createComment', $this, $data); }
        public function createProcessingInstruction(string $target, string $data): \Dom\ProcessingInstruction { return __dom('Dom\Document::createProcessingInstruction', $this, $target, $data); }
        public function importNode(?\Dom\Node $node, bool $deep = false): \Dom\Node { return __dom('Dom\Document::importNode', $this, $node, $deep); }
        public function adoptNode(\Dom\Node $node): \Dom\Node { return __dom('Dom\Document::adoptNode', $this, $node); }
        public function createAttribute(string $localName): \Dom\Attr { return __dom('Dom\Document::createAttribute', $this, $localName); }
        public function createAttributeNS(?string $namespace, string $qualifiedName): \Dom\Attr { return __dom('Dom\Document::createAttributeNS', $this, $namespace, $qualifiedName); }
        public function getElementById(string $elementId): ?\Dom\Element { return __dom('Dom\Document::getElementById', $this, $elementId); }
        public function registerNodeClass(string $baseClass, ?string $extendedClass): void { __dom('Dom\Document::registerNodeClass', $this, $baseClass, $extendedClass); }
        public function schemaValidate(string $filename, int $flags = 0): bool { return __dom('Dom\Document::schemaValidate', $this, $filename, $flags); }
        public function schemaValidateSource(string $source, int $flags = 0): bool { return __dom('Dom\Document::schemaValidateSource', $this, $source, $flags); }
        public function relaxNgValidate(string $filename): bool { return __dom('Dom\Document::relaxNgValidate', $this, $filename); }
        public function relaxNgValidateSource(string $source): bool { return __dom('Dom\Document::relaxNgValidateSource', $this, $source); }
        public function append(\Dom\Node|string ...$nodes): void { __dom('Dom\Document::append', $this, ...$nodes); }
        public function prepend(\Dom\Node|string ...$nodes): void { __dom('Dom\Document::prepend', $this, ...$nodes); }
        public function replaceChildren(\Dom\Node|string ...$nodes): void { __dom('Dom\Document::replaceChildren', $this, ...$nodes); }
        public function importLegacyNode(\DOMNode $node, bool $deep = false): \Dom\Node { return __dom('Dom\Document::importLegacyNode', $this, $node, $deep); }
        public function querySelector(string $selectors): ?\Dom\Element { return __dom('Dom\Document::querySelector', $this, $selectors); }
        public function querySelectorAll(string $selectors): \Dom\NodeList { return __dom('Dom\Document::querySelectorAll', $this, $selectors); }
    }
    final class HTMLDocument extends \Dom\Document {
        public static function createEmpty(string $encoding = 'UTF-8'): \Dom\HTMLDocument { return __dom('Dom\HTMLDocument::createEmpty', null, $encoding); }
        public static function createFromFile(string $path, int $options = 0, ?string $overrideEncoding = null): \Dom\HTMLDocument { return __dom('Dom\HTMLDocument::createFromFile', null, $path, $options, $overrideEncoding); }
        public static function createFromString(string $source, int $options = 0, ?string $overrideEncoding = null): \Dom\HTMLDocument { return __dom('Dom\HTMLDocument::createFromString', null, $source, $options, $overrideEncoding); }
        public function saveXml(?\Dom\Node $node = null, int $options = 0): string|false { return __dom('Dom\HTMLDocument::saveXml', $this, $node, $options); }
        public function saveXmlFile(string $filename, int $options = 0): int|false { return __dom('Dom\HTMLDocument::saveXmlFile', $this, $filename, $options); }
        public function saveHtml(?\Dom\Node $node = null): string { return __dom('Dom\HTMLDocument::saveHtml', $this, $node); }
        public function saveHtmlFile(string $filename): int|false { return __dom('Dom\HTMLDocument::saveHtmlFile', $this, $filename); }
    }
    final class XMLDocument extends \Dom\Document {
        public string $xmlEncoding { get => __dom_prop(0, $this, 'xmlEncoding'); set(mixed $value) { __dom_prop(1, $this, 'xmlEncoding', $value); } }
        public bool $xmlStandalone { get => __dom_prop(0, $this, 'xmlStandalone'); set(mixed $value) { __dom_prop(1, $this, 'xmlStandalone', $value); } }
        public string $xmlVersion { get => __dom_prop(0, $this, 'xmlVersion'); set(mixed $value) { __dom_prop(1, $this, 'xmlVersion', $value); } }
        public bool $formatOutput { get => __dom_prop(0, $this, 'formatOutput'); set(mixed $value) { __dom_prop(1, $this, 'formatOutput', $value); } }
        public static function createEmpty(string $version = '1.0', string $encoding = 'UTF-8'): \Dom\XMLDocument { return __dom('Dom\XMLDocument::createEmpty', null, $version, $encoding); }
        public static function createFromFile(string $path, int $options = 0, ?string $overrideEncoding = null): \Dom\XMLDocument { return __dom('Dom\XMLDocument::createFromFile', null, $path, $options, $overrideEncoding); }
        public static function createFromString(string $source, int $options = 0, ?string $overrideEncoding = null): \Dom\XMLDocument { return __dom('Dom\XMLDocument::createFromString', null, $source, $options, $overrideEncoding); }
        public function createEntityReference(string $name): \Dom\EntityReference { return __dom('Dom\XMLDocument::createEntityReference', $this, $name); }
        public function validate(): bool { return __dom('Dom\XMLDocument::validate', $this); }
        public function xinclude(int $options = 0): int { return __dom('Dom\XMLDocument::xinclude', $this, $options); }
        public function saveXml(?\Dom\Node $node = null, int $options = 0): string|false { return __dom('Dom\XMLDocument::saveXml', $this, $node, $options); }
        public function saveXmlFile(string $filename, int $options = 0): int|false { return __dom('Dom\XMLDocument::saveXmlFile', $this, $filename, $options); }
    }
    class NodeList implements \IteratorAggregate, \Countable {
        public int $length { get => __dom_prop(0, $this, 'length'); set(mixed $value) { __dom_prop(1, $this, 'length', $value); } }
        public function count(): int { return __dom('Dom\NodeList::count', $this); }
        public function getIterator(): \Iterator { return __dom('Dom\NodeList::getIterator', $this); }
        public function item(int $index): ?\Dom\Node { return __dom('Dom\NodeList::item', $this, $index); }
    }
    class NamedNodeMap implements \IteratorAggregate, \Countable {
        public int $length { get => __dom_prop(0, $this, 'length'); set(mixed $value) { __dom_prop(1, $this, 'length', $value); } }
        public function item(int $index): ?\Dom\Attr { return __dom('Dom\NamedNodeMap::item', $this, $index); }
        public function getNamedItem(string $qualifiedName): ?\Dom\Attr { return __dom('Dom\NamedNodeMap::getNamedItem', $this, $qualifiedName); }
        public function getNamedItemNS(?string $namespace, string $localName): ?\Dom\Attr { return __dom('Dom\NamedNodeMap::getNamedItemNS', $this, $namespace, $localName); }
        public function count(): int { return __dom('Dom\NamedNodeMap::count', $this); }
        public function getIterator(): \Iterator { return __dom('Dom\NamedNodeMap::getIterator', $this); }
    }
    class DtdNamedNodeMap implements \IteratorAggregate, \Countable {
        public int $length { get => __dom_prop(0, $this, 'length'); set(mixed $value) { __dom_prop(1, $this, 'length', $value); } }
        public function item(int $index): \Dom\Entity|\Dom\Notation|null { return __dom('Dom\DtdNamedNodeMap::item', $this, $index); }
        public function getNamedItem(string $qualifiedName): \Dom\Entity|\Dom\Notation|null { return __dom('Dom\DtdNamedNodeMap::getNamedItem', $this, $qualifiedName); }
        public function getNamedItemNS(?string $namespace, string $localName): \Dom\Entity|\Dom\Notation|null { return __dom('Dom\DtdNamedNodeMap::getNamedItemNS', $this, $namespace, $localName); }
        public function count(): int { return __dom('Dom\DtdNamedNodeMap::count', $this); }
        public function getIterator(): \Iterator { return __dom('Dom\DtdNamedNodeMap::getIterator', $this); }
    }
    class HTMLCollection implements \IteratorAggregate, \Countable {
        public int $length { get => __dom_prop(0, $this, 'length'); set(mixed $value) { __dom_prop(1, $this, 'length', $value); } }
        public function item(int $index): ?\Dom\Element { return __dom('Dom\HTMLCollection::item', $this, $index); }
        public function namedItem(string $key): ?\Dom\Element { return __dom('Dom\HTMLCollection::namedItem', $this, $key); }
        public function count(): int { return __dom('Dom\HTMLCollection::count', $this); }
        public function getIterator(): \Iterator { return __dom('Dom\HTMLCollection::getIterator', $this); }
    }
    class CharacterData extends \Dom\Node implements \Dom\ChildNode {
        public ?\Dom\Element $previousElementSibling { get => __dom_prop(0, $this, 'previousElementSibling'); set(mixed $value) { __dom_prop(1, $this, 'previousElementSibling', $value); } }
        public ?\Dom\Element $nextElementSibling { get => __dom_prop(0, $this, 'nextElementSibling'); set(mixed $value) { __dom_prop(1, $this, 'nextElementSibling', $value); } }
        public string $data { get => __dom_prop(0, $this, 'data'); set(mixed $value) { __dom_prop(1, $this, 'data', $value); } }
        public int $length { get => __dom_prop(0, $this, 'length'); set(mixed $value) { __dom_prop(1, $this, 'length', $value); } }
        public function substringData(int $offset, int $count): string { return __dom('Dom\CharacterData::substringData', $this, $offset, $count); }
        public function appendData(string $data): void { __dom('Dom\CharacterData::appendData', $this, $data); }
        public function insertData(int $offset, string $data): void { __dom('Dom\CharacterData::insertData', $this, $offset, $data); }
        public function deleteData(int $offset, int $count): void { __dom('Dom\CharacterData::deleteData', $this, $offset, $count); }
        public function replaceData(int $offset, int $count, string $data): void { __dom('Dom\CharacterData::replaceData', $this, $offset, $count, $data); }
        public function remove(): void { __dom('Dom\CharacterData::remove', $this); }
        public function before(\Dom\Node|string ...$nodes): void { __dom('Dom\CharacterData::before', $this, ...$nodes); }
        public function after(\Dom\Node|string ...$nodes): void { __dom('Dom\CharacterData::after', $this, ...$nodes); }
        public function replaceWith(\Dom\Node|string ...$nodes): void { __dom('Dom\CharacterData::replaceWith', $this, ...$nodes); }
    }
    class Attr extends \Dom\Node {
        public ?string $namespaceURI { get => __dom_prop(0, $this, 'namespaceURI'); set(mixed $value) { __dom_prop(1, $this, 'namespaceURI', $value); } }
        public ?string $prefix { get => __dom_prop(0, $this, 'prefix'); set(mixed $value) { __dom_prop(1, $this, 'prefix', $value); } }
        public string $localName { get => __dom_prop(0, $this, 'localName'); set(mixed $value) { __dom_prop(1, $this, 'localName', $value); } }
        public string $name { get => __dom_prop(0, $this, 'name'); set(mixed $value) { __dom_prop(1, $this, 'name', $value); } }
        public string $value { get => __dom_prop(0, $this, 'value'); set(mixed $value) { __dom_prop(1, $this, 'value', $value); } }
        public ?\Dom\Element $ownerElement { get => __dom_prop(0, $this, 'ownerElement'); set(mixed $value) { __dom_prop(1, $this, 'ownerElement', $value); } }
        public bool $specified { get => __dom_prop(0, $this, 'specified'); set(mixed $value) { __dom_prop(1, $this, 'specified', $value); } }
        public function isId(): bool { return __dom('Dom\Attr::isId', $this); }
        public function rename(?string $namespaceURI, string $qualifiedName): void { __dom('Dom\Attr::rename', $this, $namespaceURI, $qualifiedName); }
    }
    class Element extends \Dom\Node implements \Dom\ParentNode, \Dom\ChildNode {
        public ?string $namespaceURI { get => __dom_prop(0, $this, 'namespaceURI'); set(mixed $value) { __dom_prop(1, $this, 'namespaceURI', $value); } }
        public ?string $prefix { get => __dom_prop(0, $this, 'prefix'); set(mixed $value) { __dom_prop(1, $this, 'prefix', $value); } }
        public string $localName { get => __dom_prop(0, $this, 'localName'); set(mixed $value) { __dom_prop(1, $this, 'localName', $value); } }
        public string $tagName { get => __dom_prop(0, $this, 'tagName'); set(mixed $value) { __dom_prop(1, $this, 'tagName', $value); } }
        public \Dom\HTMLCollection $children { get => $this->children ??= __dom_prop(0, $this, 'children'); set(mixed $value) { __dom_prop(1, $this, 'children', $value); } }
        public ?\Dom\Element $firstElementChild { get => __dom_prop(0, $this, 'firstElementChild'); set(mixed $value) { __dom_prop(1, $this, 'firstElementChild', $value); } }
        public ?\Dom\Element $lastElementChild { get => __dom_prop(0, $this, 'lastElementChild'); set(mixed $value) { __dom_prop(1, $this, 'lastElementChild', $value); } }
        public int $childElementCount { get => __dom_prop(0, $this, 'childElementCount'); set(mixed $value) { __dom_prop(1, $this, 'childElementCount', $value); } }
        public ?\Dom\Element $previousElementSibling { get => __dom_prop(0, $this, 'previousElementSibling'); set(mixed $value) { __dom_prop(1, $this, 'previousElementSibling', $value); } }
        public ?\Dom\Element $nextElementSibling { get => __dom_prop(0, $this, 'nextElementSibling'); set(mixed $value) { __dom_prop(1, $this, 'nextElementSibling', $value); } }
        public string $id { get => __dom_prop(0, $this, 'id'); set(mixed $value) { __dom_prop(1, $this, 'id', $value); } }
        public string $className { get => __dom_prop(0, $this, 'className'); set(mixed $value) { __dom_prop(1, $this, 'className', $value); } }
        public \Dom\TokenList $classList { get => $this->classList ??= __dom_prop(0, $this, 'classList'); set(mixed $value) { __dom_prop(1, $this, 'classList', $value); } }
        public \Dom\NamedNodeMap $attributes { get => __dom_prop(0, $this, 'attributes'); set(mixed $value) { __dom_prop(1, $this, 'attributes', $value); } }
        public string $innerHTML { get => __dom_prop(0, $this, 'innerHTML'); set(mixed $value) { __dom_prop(1, $this, 'innerHTML', $value); } }
        public string $outerHTML { get => __dom_prop(0, $this, 'outerHTML'); set(mixed $value) { __dom_prop(1, $this, 'outerHTML', $value); } }
        public string $substitutedNodeValue { get => __dom_prop(0, $this, 'substitutedNodeValue'); set(mixed $value) { __dom_prop(1, $this, 'substitutedNodeValue', $value); } }
        public function hasAttributes(): bool { return __dom('Dom\Element::hasAttributes', $this); }
        public function getAttributeNames(): array { return __dom('Dom\Element::getAttributeNames', $this); }
        public function getAttribute(string $qualifiedName): ?string { return __dom('Dom\Element::getAttribute', $this, $qualifiedName); }
        public function getAttributeNS(?string $namespace, string $localName): ?string { return __dom('Dom\Element::getAttributeNS', $this, $namespace, $localName); }
        public function setAttribute(string $qualifiedName, string $value): void { __dom('Dom\Element::setAttribute', $this, $qualifiedName, $value); }
        public function setAttributeNS(?string $namespace, string $qualifiedName, string $value): void { __dom('Dom\Element::setAttributeNS', $this, $namespace, $qualifiedName, $value); }
        public function removeAttribute(string $qualifiedName): void { __dom('Dom\Element::removeAttribute', $this, $qualifiedName); }
        public function removeAttributeNS(?string $namespace, string $localName): void { __dom('Dom\Element::removeAttributeNS', $this, $namespace, $localName); }
        public function toggleAttribute(string $qualifiedName, ?bool $force = null): bool { return __dom('Dom\Element::toggleAttribute', $this, $qualifiedName, $force); }
        public function hasAttribute(string $qualifiedName): bool { return __dom('Dom\Element::hasAttribute', $this, $qualifiedName); }
        public function hasAttributeNS(?string $namespace, string $localName): bool { return __dom('Dom\Element::hasAttributeNS', $this, $namespace, $localName); }
        public function getAttributeNode(string $qualifiedName): ?\Dom\Attr { return __dom('Dom\Element::getAttributeNode', $this, $qualifiedName); }
        public function getAttributeNodeNS(?string $namespace, string $localName): ?\Dom\Attr { return __dom('Dom\Element::getAttributeNodeNS', $this, $namespace, $localName); }
        public function setAttributeNode(\Dom\Attr $attr): ?\Dom\Attr { return __dom('Dom\Element::setAttributeNode', $this, $attr); }
        public function setAttributeNodeNS(\Dom\Attr $attr): ?\Dom\Attr { return __dom('Dom\Element::setAttributeNodeNS', $this, $attr); }
        public function removeAttributeNode(\Dom\Attr $attr): \Dom\Attr { return __dom('Dom\Element::removeAttributeNode', $this, $attr); }
        public function getElementsByTagName(string $qualifiedName): \Dom\HTMLCollection { return __dom('Dom\Element::getElementsByTagName', $this, $qualifiedName); }
        public function getElementsByTagNameNS(?string $namespace, string $localName): \Dom\HTMLCollection { return __dom('Dom\Element::getElementsByTagNameNS', $this, $namespace, $localName); }
        public function getElementsByClassName(string $classNames): \Dom\HTMLCollection { return __dom('Dom\Element::getElementsByClassName', $this, $classNames); }
        public function insertAdjacentElement(\Dom\AdjacentPosition $where, \Dom\Element $element): ?\Dom\Element { return __dom('Dom\Element::insertAdjacentElement', $this, $where, $element); }
        public function insertAdjacentText(\Dom\AdjacentPosition $where, string $data): void { __dom('Dom\Element::insertAdjacentText', $this, $where, $data); }
        public function insertAdjacentHTML(\Dom\AdjacentPosition $where, string $string): void { __dom('Dom\Element::insertAdjacentHTML', $this, $where, $string); }
        public function setIdAttribute(string $qualifiedName, bool $isId): void { __dom('Dom\Element::setIdAttribute', $this, $qualifiedName, $isId); }
        public function setIdAttributeNS(?string $namespace, string $qualifiedName, bool $isId): void { __dom('Dom\Element::setIdAttributeNS', $this, $namespace, $qualifiedName, $isId); }
        public function setIdAttributeNode(\Dom\Attr $attr, bool $isId): void { __dom('Dom\Element::setIdAttributeNode', $this, $attr, $isId); }
        public function remove(): void { __dom('Dom\Element::remove', $this); }
        public function before(\Dom\Node|string ...$nodes): void { __dom('Dom\Element::before', $this, ...$nodes); }
        public function after(\Dom\Node|string ...$nodes): void { __dom('Dom\Element::after', $this, ...$nodes); }
        public function replaceWith(\Dom\Node|string ...$nodes): void { __dom('Dom\Element::replaceWith', $this, ...$nodes); }
        public function append(\Dom\Node|string ...$nodes): void { __dom('Dom\Element::append', $this, ...$nodes); }
        public function prepend(\Dom\Node|string ...$nodes): void { __dom('Dom\Element::prepend', $this, ...$nodes); }
        public function replaceChildren(\Dom\Node|string ...$nodes): void { __dom('Dom\Element::replaceChildren', $this, ...$nodes); }
        public function querySelector(string $selectors): ?\Dom\Element { return __dom('Dom\Element::querySelector', $this, $selectors); }
        public function querySelectorAll(string $selectors): \Dom\NodeList { return __dom('Dom\Element::querySelectorAll', $this, $selectors); }
        public function closest(string $selectors): ?\Dom\Element { return __dom('Dom\Element::closest', $this, $selectors); }
        public function matches(string $selectors): bool { return __dom('Dom\Element::matches', $this, $selectors); }
        public function getInScopeNamespaces(): array { return __dom('Dom\Element::getInScopeNamespaces', $this); }
        public function getDescendantNamespaces(): array { return __dom('Dom\Element::getDescendantNamespaces', $this); }
        public function rename(?string $namespaceURI, string $qualifiedName): void { __dom('Dom\Element::rename', $this, $namespaceURI, $qualifiedName); }
    }
    class HTMLElement extends \Dom\Element {
    }
    class Text extends \Dom\CharacterData {
        public string $wholeText { get => __dom_prop(0, $this, 'wholeText'); set(mixed $value) { __dom_prop(1, $this, 'wholeText', $value); } }
        public function splitText(int $offset): \Dom\Text { return __dom('Dom\Text::splitText', $this, $offset); }
    }
    class Comment extends \Dom\CharacterData {
    }
    class CDATASection extends \Dom\Text {
    }
    class DocumentType extends \Dom\Node implements \Dom\ChildNode {
        public string $name { get => __dom_prop(0, $this, 'name'); set(mixed $value) { __dom_prop(1, $this, 'name', $value); } }
        public \Dom\DtdNamedNodeMap $entities { get => __dom_prop(0, $this, 'entities'); set(mixed $value) { __dom_prop(1, $this, 'entities', $value); } }
        public \Dom\DtdNamedNodeMap $notations { get => __dom_prop(0, $this, 'notations'); set(mixed $value) { __dom_prop(1, $this, 'notations', $value); } }
        public string $publicId { get => __dom_prop(0, $this, 'publicId'); set(mixed $value) { __dom_prop(1, $this, 'publicId', $value); } }
        public string $systemId { get => __dom_prop(0, $this, 'systemId'); set(mixed $value) { __dom_prop(1, $this, 'systemId', $value); } }
        public ?string $internalSubset { get => __dom_prop(0, $this, 'internalSubset'); set(mixed $value) { __dom_prop(1, $this, 'internalSubset', $value); } }
        public function remove(): void { __dom('Dom\DocumentType::remove', $this); }
        public function before(\Dom\Node|string ...$nodes): void { __dom('Dom\DocumentType::before', $this, ...$nodes); }
        public function after(\Dom\Node|string ...$nodes): void { __dom('Dom\DocumentType::after', $this, ...$nodes); }
        public function replaceWith(\Dom\Node|string ...$nodes): void { __dom('Dom\DocumentType::replaceWith', $this, ...$nodes); }
    }
    class Notation extends \Dom\Node {
        public string $publicId { get => __dom_prop(0, $this, 'publicId'); set(mixed $value) { __dom_prop(1, $this, 'publicId', $value); } }
        public string $systemId { get => __dom_prop(0, $this, 'systemId'); set(mixed $value) { __dom_prop(1, $this, 'systemId', $value); } }
    }
    class Entity extends \Dom\Node {
        public ?string $publicId { get => __dom_prop(0, $this, 'publicId'); set(mixed $value) { __dom_prop(1, $this, 'publicId', $value); } }
        public ?string $systemId { get => __dom_prop(0, $this, 'systemId'); set(mixed $value) { __dom_prop(1, $this, 'systemId', $value); } }
        public ?string $notationName { get => __dom_prop(0, $this, 'notationName'); set(mixed $value) { __dom_prop(1, $this, 'notationName', $value); } }
    }
    class EntityReference extends \Dom\Node {
    }
    class ProcessingInstruction extends \Dom\CharacterData {
        public string $target { get => __dom_prop(0, $this, 'target'); set(mixed $value) { __dom_prop(1, $this, 'target', $value); } }
    }
    final class XPath {
        public \Dom\Document $document { get => __dom_prop(0, $this, 'document'); set(mixed $value) { __dom_prop(1, $this, 'document', $value); } }
        public bool $registerNodeNamespaces { get => __dom_prop(0, $this, 'registerNodeNamespaces'); set(mixed $value) { __dom_prop(1, $this, 'registerNodeNamespaces', $value); } }
        public function __construct(\Dom\Document $document, bool $registerNodeNS = true) { __dom('Dom\XPath::__construct', $this, $document, $registerNodeNS); }
        public function evaluate(string $expression, ?\Dom\Node $contextNode = null, bool $registerNodeNS = true): \Dom\NodeList|string|float|bool|null { return __dom('Dom\XPath::evaluate', $this, $expression, $contextNode, $registerNodeNS); }
        public function query(string $expression, ?\Dom\Node $contextNode = null, bool $registerNodeNS = true): \Dom\NodeList { return __dom('Dom\XPath::query', $this, $expression, $contextNode, $registerNodeNS); }
        public function registerNamespace(string $prefix, string $namespace): bool { return __dom('Dom\XPath::registerNamespace', $this, $prefix, $namespace); }
        public function registerPhpFunctions(array|string|null $restrict = null): void { __dom('Dom\XPath::registerPhpFunctions', $this, $restrict); }
        public function registerPhpFunctionNS(string $namespaceURI, string $name, callable $callable): void { __dom('Dom\XPath::registerPhpFunctionNS', $this, $namespaceURI, $name, $callable); }
        public static function quote(string $str): string { return __dom('Dom\XPath::quote', null, $str); }
    }
    final class TokenList implements \IteratorAggregate, \Countable {
        public int $length { get => __dom_prop(0, $this, 'length'); set(mixed $value) { __dom_prop(1, $this, 'length', $value); } }
        public string $value { get => __dom_prop(0, $this, 'value'); set(mixed $value) { __dom_prop(1, $this, 'value', $value); } }
        private function __construct() { throw new \Error('Cannot directly construct ' . static::class . ', use document methods instead'); }
        public function item(int $index): ?string { return __dom('Dom\TokenList::item', $this, $index); }
        public function contains(string $token): bool { return __dom('Dom\TokenList::contains', $this, $token); }
        public function add(...$tokens): void { __dom('Dom\TokenList::add', $this, ...$tokens); }
        public function remove(...$tokens): void { __dom('Dom\TokenList::remove', $this, ...$tokens); }
        public function toggle(string $token, ?bool $force = null): bool { return __dom('Dom\TokenList::toggle', $this, $token, $force); }
        public function replace(string $token, string $newToken): bool { return __dom('Dom\TokenList::replace', $this, $token, $newToken); }
        public function supports(string $token): bool { return __dom('Dom\TokenList::supports', $this, $token); }
        public function count(): int { return __dom('Dom\TokenList::count', $this); }
        public function getIterator(): \Iterator { return __dom('Dom\TokenList::getIterator', $this); }
    }
}
