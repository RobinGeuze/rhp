
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

    public $__d;
    public $__n;

    public function __get($name) {
        if ($this->__d === null) { throw new DOMException('Invalid State Error', 11); }
        $v = __dom('get', $this->__d, $this->__n, $name);
        if (is_array($v) && isset($v['__unknown'])) { return $this->$name; }
        return $v;
    }
    public function __set($name, $value) {
        if ($this->__d === null) { throw new DOMException('Invalid State Error', 11); }
        $r = __dom('set', $this->__d, $this->__n, $name, $value);
        if ($r === 'ro') { throw new Error('Cannot modify readonly property ' . static::class . '::$' . $name); }
        if ($r === false) { __dom('dynprop', static::class, $name); $this->$name = $value; }
    }
    public function __isset($name) { return __dom('isset', $this->__d, $this->__n, $name); }
    public function __unset($name) {}

    public function appendChild(DOMNode $node) { return __dom('append', $this->__d, $this->__n, $node); }
    public function insertBefore(DOMNode $node, ?DOMNode $child = null) { return __dom('insertBefore', $this->__d, $this->__n, $node, $child); }
    public function removeChild(DOMNode $child) { return __dom('removeChild', $this->__d, $this->__n, $child); }
    public function replaceChild(DOMNode $node, DOMNode $child) { return __dom('replaceChild', $this->__d, $this->__n, $node, $child); }
    public function cloneNode(bool $deep = false) { return __dom('clone', $this->__d, $this->__n, $deep); }
    public function hasChildNodes(): bool { return __dom('hasChildNodes', $this->__d, $this->__n); }
    public function hasAttributes(): bool { return __dom('hasAttributes', $this->__d, $this->__n); }
    public function normalize(): void { __dom('normalize', $this->__d, $this->__n); }
    public function isSameNode(DOMNode $otherNode): bool { return __dom('isSame', $this->__d, $this->__n, $otherNode); }
    public function isEqualNode(?DOMNode $otherNode): bool { return $otherNode !== null && __dom('isEqual', $this->__d, $this->__n, $otherNode); }
    public function isSupported(string $feature, string $version): bool { return true; }
    public function lookupNamespaceURI(?string $prefix): ?string { return __dom('lookupNamespaceURI', $this->__d, $this->__n, $prefix); }
    public function lookupPrefix(string $namespace): ?string { return __dom('lookupPrefix', $this->__d, $this->__n, $namespace); }
    public function isDefaultNamespace(string $namespace): bool { return __dom('isDefaultNamespace', $this->__d, $this->__n, $namespace); }
    public function getNodePath(): ?string { return __dom('nodePath', $this->__d, $this->__n); }
    public function getLineNo(): int { return __dom('lineNo', $this->__d, $this->__n); }
    public function contains($other): bool { return $other !== null && __dom('contains', $this->__d, $this->__n, $other); }
    public function getRootNode(?array $options = null): DOMNode { return __dom('rootNode', $this->__d, $this->__n); }
    public function C14N(bool $exclusive = false, bool $withComments = false, ?array $xpath = null, ?array $nsPrefixes = null): string|false { return __dom('c14n', $this->__d, $this->__n); }
    public function C14NFile(string $uri, bool $exclusive = false, bool $withComments = false, ?array $xpath = null, ?array $nsPrefixes = null): int|false {
        $s = $this->C14N(); return file_put_contents($uri, $s);
    }
    public function compareDocumentPosition(DOMNode $other): int {
        if ($this->isSameNode($other)) { return 0; }
        if ($this->__d !== $other->__d) { return 1 | 32 | 2; }
        if ($this->contains($other)) { return 16 | 4; }
        if ($other->contains($this)) { return 8 | 2; }
        $p = __dom('nodePath', $this->__d, $this->__n); $q = __dom('nodePath', $other->__d, $other->__n);
        return strcmp($p, $q) < 0 ? 4 : 2;
    }
    public function __sleep(): array { throw new Exception("Serialization of '" . static::class . "' is not allowed, unless serialization methods are implemented in a subclass"); }
    public function __wakeup(): void { throw new Exception("Unserialization of '" . static::class . "' is not allowed, unless unserialization methods are implemented in a subclass"); }

    // ParentNode / ChildNode helpers
    protected function __nodes(array $nodes) {
        $doc = $this instanceof DOMDocument ? $this : $this->ownerDocument;
        $out = [];
        foreach ($nodes as $n) { $out[] = is_string($n) ? $doc->createTextNode($n) : $n; }
        return $out;
    }
}

class DOMNameSpaceNode {
    public $__d; public $__n;
}

class DOMImplementation {
    public function hasFeature(string $feature, string $version): bool { return true; }
    public function createDocumentType(string $qualifiedName, string $publicId = "", string $systemId = "") { return false; }
    public function createDocument(?string $namespace = null, string $qualifiedName = "", ?DOMDocumentType $doctype = null): DOMDocument {
        $d = new DOMDocument();
        if ($qualifiedName !== '') {
            $d->appendChild($namespace === null ? $d->createElement($qualifiedName) : $d->createElementNS($namespace, $qualifiedName));
        }
        return $d;
    }
}

class DOMDocumentFragment extends DOMNode implements DOMParentNode {
    public function __construct() { __dom('newnode', $this, 'fragment'); }
    public function appendXML(string $data): bool { return __dom('appendXML', $this->__d, $this->__n, $data); }
    public function append(...$nodes): void { foreach ($this->__nodes($nodes) as $n) { $this->appendChild($n); } }
    public function prepend(...$nodes): void { $f = $this->firstChild; foreach ($this->__nodes($nodes) as $n) { $this->insertBefore($n, $f); } }
    public function replaceChildren(...$nodes): void { while ($this->firstChild) { $this->removeChild($this->firstChild); } $this->append(...$nodes); }
}

class DOMNodeList implements IteratorAggregate, Countable, ArrayAccess {
    public $__d; public $__n; public $__m; public $__a; public $__b;
    public function __get($name) {
        if ($name === 'length') { return __dom('listLen', $this->__d, $this->__n, $this->__m, $this->__a, $this->__b); }
        return $this->$name;
    }
    public function __isset($name) { return $name === 'length'; }
    public function count(): int { return __dom('listLen', $this->__d, $this->__n, $this->__m, $this->__a, $this->__b); }
    public function item(int $index) { return __dom('listItem', $this->__d, $this->__n, $this->__m, $this->__a, $this->__b, $index); }
    // dimension reads (php's read_dimension handler): numeric offsets only
    public function offsetGet($o): mixed {
        if (is_string($o) && !is_numeric($o)) { return null; }
        return $this->item((int) $o);
    }
    public function offsetExists($o): bool {
        if (is_string($o) && !is_numeric($o)) { return false; }
        $o = (int) $o;
        return $o >= 0 && $o < $this->count();
    }
    public function offsetSet($o, $v): void { throw new Error('Cannot use object of type ' . static::class . ' as array'); }
    public function offsetUnset($o): void { throw new Error('Cannot use object of type ' . static::class . ' as array'); }
    public function getIterator(): Iterator {
        $i = 0;
        while (($n = __dom('listItem', $this->__d, $this->__n, $this->__m, $this->__a, $this->__b, $i)) !== null) { yield $i => $n; $i++; }
    }
}

class DOMNamedNodeMap implements IteratorAggregate, Countable, ArrayAccess {
    public $__d; public $__n;
    public function __get($name) {
        if ($name === 'length') { return __dom('mapLen', $this->__d, $this->__n); }
        return $this->$name;
    }
    public function __isset($name) { return $name === 'length'; }
    public function getNamedItem(string $qualifiedName): ?DOMNode { return __dom('mapNamed', $this->__d, $this->__n, $qualifiedName); }
    public function getNamedItemNS(?string $namespace, string $localName): ?DOMNode { return __dom('mapNamedNS', $this->__d, $this->__n, $namespace, $localName); }
    public function item(int $index): ?DOMNode { return __dom('mapItem', $this->__d, $this->__n, $index); }
    public function count(): int { return __dom('mapLen', $this->__d, $this->__n); }
    // dimension reads: names look up attributes, numbers index them
    public function offsetGet($o): mixed {
        if (is_string($o) && !is_numeric($o)) { return $this->getNamedItem($o); }
        return $this->item((int) $o);
    }
    public function offsetExists($o): bool {
        if (is_string($o) && !is_numeric($o)) { return $this->getNamedItem($o) !== null; }
        $o = (int) $o;
        return $o >= 0 && $o < $this->count();
    }
    public function offsetSet($o, $v): void { throw new Error('Cannot use object of type ' . static::class . ' as array'); }
    public function offsetUnset($o): void { throw new Error('Cannot use object of type ' . static::class . ' as array'); }
    public function getIterator(): Iterator {
        $len = __dom('mapLen', $this->__d, $this->__n);
        for ($i = 0; $i < $len; $i++) { $a = __dom('mapItem', $this->__d, $this->__n, $i); if ($a === null) { break; } yield $a->localName => $a; }
    }
}

class DOMCharacterData extends DOMNode implements DOMChildNode {
    public function appendData(string $data): true { __dom('appendData', $this->__d, $this->__n, $data); return true; }
    public function substringData(int $offset, int $count) { return __dom('substringData', $this->__d, $this->__n, $offset, $count); }
    public function insertData(int $offset, string $data): bool { return __dom('insertData', $this->__d, $this->__n, $offset, $data); }
    public function deleteData(int $offset, int $count): bool { return __dom('deleteData', $this->__d, $this->__n, $offset, $count); }
    public function replaceData(int $offset, int $count, string $data): bool { return __dom('replaceData', $this->__d, $this->__n, $offset, $count, $data); }
    public function remove(): void { __dom('remove', $this->__d, $this->__n); }
    public function before(...$nodes): void { $p = $this->parentNode; if ($p) { foreach ($this->__nodes($nodes) as $n) { $p->insertBefore($n, $this); } } }
    public function after(...$nodes): void { $p = $this->parentNode; if ($p) { $r = $this->nextSibling; foreach ($this->__nodes($nodes) as $n) { $p->insertBefore($n, $r); } } }
    public function replaceWith(...$nodes): void { $p = $this->parentNode; if ($p) { $r = $this->nextSibling; $p->removeChild($this); foreach ($this->__nodes($nodes) as $n) { $p->insertBefore($n, $r); } } }
}

class DOMText extends DOMCharacterData {
    public function __construct(string $data = "") { __dom('newnode', $this, 'text', $data); }
    public function isWhitespaceInElementContent(): bool { return trim($this->data, " \t\r\n") === ''; }
    public function isElementContentWhitespace(): bool { return $this->isWhitespaceInElementContent(); }
    public function splitText(int $offset) { return __dom('splitText', $this->__d, $this->__n, $offset); }
}

class DOMCdataSection extends DOMText {
    public function __construct(string $data) { __dom('newnode', $this, 'cdata', $data); }
}

class DOMComment extends DOMCharacterData {
    public function __construct(string $data = "") { __dom('newnode', $this, 'comment', $data); }
}

class DOMAttr extends DOMNode {
    public function __construct(string $name, string $value = "") { __dom('newnode', $this, 'attr', $name, $value); }
    public function isId(): bool { return $this->localName === 'id' && $this->prefix === 'xml'; }
}

class DOMElement extends DOMNode implements DOMParentNode, DOMChildNode {
    public function __construct(string $qualifiedName, ?string $value = null, string $namespace = "") { __dom('newnode', $this, 'element', $qualifiedName, $value, $namespace); }
    public function getAttribute(string $qualifiedName): string { return __dom('getAttribute', $this->__d, $this->__n, $qualifiedName); }
    public function getAttributeNames(): array { return __dom('attrNames', $this->__d, $this->__n); }
    public function getAttributeNS(?string $namespace, string $localName): string { return __dom('getAttributeNS', $this->__d, $this->__n, $namespace, $localName); }
    public function getAttributeNode(string $qualifiedName) { return __dom('getAttributeNode', $this->__d, $this->__n, $qualifiedName); }
    public function getAttributeNodeNS(?string $namespace, string $localName) { return __dom('getAttributeNodeNS', $this->__d, $this->__n, $namespace, $localName); }
    public function getElementsByTagName(string $qualifiedName): DOMNodeList { return __dom('byTag', $this->__d, $this->__n, $qualifiedName); }
    public function getElementsByTagNameNS(?string $namespace, string $localName): DOMNodeList { return __dom('byTagNS', $this->__d, $this->__n, $namespace, $localName); }
    public function hasAttribute(string $qualifiedName): bool { return __dom('hasAttribute', $this->__d, $this->__n, $qualifiedName); }
    public function hasAttributeNS(?string $namespace, string $localName): bool { return __dom('hasAttributeNS', $this->__d, $this->__n, $namespace, $localName); }
    public function removeAttribute(string $qualifiedName): bool { return __dom('removeAttribute', $this->__d, $this->__n, $qualifiedName); }
    public function removeAttributeNS(?string $namespace, string $localName): void { __dom('removeAttributeNS', $this->__d, $this->__n, $namespace, $localName); }
    public function removeAttributeNode(DOMAttr $attr) { return __dom('removeAttributeNode', $this->__d, $this->__n, $attr); }
    public function setAttribute(string $qualifiedName, string $value) { return __dom('setAttribute', $this->__d, $this->__n, $qualifiedName, $value); }
    public function setAttributeNS(?string $namespace, string $qualifiedName, string $value): void { __dom('setAttributeNS', $this->__d, $this->__n, $namespace, $qualifiedName, $value); }
    public function setAttributeNode(DOMAttr $attr) { return __dom('setAttributeNode', $this->__d, $this->__n, $attr); }
    public function setAttributeNodeNS(DOMAttr $attr) { return __dom('setAttributeNode', $this->__d, $this->__n, $attr); }
    public function setIdAttribute(string $qualifiedName, bool $isId): void {}
    public function setIdAttributeNS(string $namespace, string $qualifiedName, bool $isId): void {}
    public function setIdAttributeNode(DOMAttr $attr, bool $isId): void {}
    public function toggleAttribute(string $qualifiedName, ?bool $force = null): bool {
        if ($this->hasAttribute($qualifiedName)) {
            if ($force === true) { return true; }
            $this->removeAttribute($qualifiedName); return false;
        }
        if ($force === false) { return false; }
        $this->setAttribute($qualifiedName, ''); return true;
    }
    public function remove(): void { __dom('remove', $this->__d, $this->__n); }
    public function before(...$nodes): void { $p = $this->parentNode; if ($p) { foreach ($this->__nodes($nodes) as $n) { $p->insertBefore($n, $this); } } }
    public function after(...$nodes): void { $p = $this->parentNode; if ($p) { $r = $this->nextSibling; foreach ($this->__nodes($nodes) as $n) { $p->insertBefore($n, $r); } } }
    public function replaceWith(...$nodes): void { $p = $this->parentNode; if ($p) { $r = $this->nextSibling; $p->removeChild($this); foreach ($this->__nodes($nodes) as $n) { $p->insertBefore($n, $r); } } }
    public function append(...$nodes): void { foreach ($this->__nodes($nodes) as $n) { $this->appendChild($n); } }
    public function prepend(...$nodes): void { $f = $this->firstChild; foreach ($this->__nodes($nodes) as $n) { $this->insertBefore($n, $f); } }
    public function replaceChildren(...$nodes): void { while ($this->firstChild) { $this->removeChild($this->firstChild); } $this->append(...$nodes); }
    public function insertAdjacentElement(string $where, DOMElement $element): ?DOMElement {
        switch (strtolower($where)) {
            case 'beforebegin': if (!$this->parentNode) { return null; } $this->parentNode->insertBefore($element, $this); return $element;
            case 'afterbegin': $this->insertBefore($element, $this->firstChild); return $element;
            case 'beforeend': $this->appendChild($element); return $element;
            case 'afterend': if (!$this->parentNode) { return null; } $this->parentNode->insertBefore($element, $this->nextSibling); return $element;
        }
        throw new DOMException('Syntax Error', 12);
    }
    public function insertAdjacentText(string $where, string $data): void {
        $this->insertAdjacentElement2($where, $this->ownerDocument->createTextNode($data));
    }
    private function insertAdjacentElement2($where, $node) {
        switch (strtolower($where)) {
            case 'beforebegin': if ($this->parentNode) { $this->parentNode->insertBefore($node, $this); } return;
            case 'afterbegin': $this->insertBefore($node, $this->firstChild); return;
            case 'beforeend': $this->appendChild($node); return;
            case 'afterend': if ($this->parentNode) { $this->parentNode->insertBefore($node, $this->nextSibling); } return;
        }
        throw new DOMException('Syntax Error', 12);
    }
}

class DOMDocument extends DOMNode implements DOMParentNode {
    public function __construct(string $version = "1.0", string $encoding = "") { __dom('newdoc', $this, $version, $encoding); }
    public function createAttribute(string $localName) { return __dom('create', $this->__d, 'attr', $localName); }
    public function createAttributeNS(?string $namespace, string $qualifiedName) { return __dom('create', $this->__d, 'attrNS', $namespace, $qualifiedName); }
    public function createCDATASection(string $data) { return __dom('create', $this->__d, 'cdata', $data); }
    public function createComment(string $data): DOMComment { return __dom('create', $this->__d, 'comment', $data); }
    public function createDocumentFragment(): DOMDocumentFragment { return __dom('create', $this->__d, 'fragment'); }
    public function createElement(string $localName, string $value = "") { return __dom('create', $this->__d, 'element', $localName, $value); }
    public function createElementNS(?string $namespace, string $qualifiedName, string $value = "") { return __dom('create', $this->__d, 'elementNS', $namespace, $qualifiedName, $value); }
    public function createEntityReference(string $name) { return __dom('create', $this->__d, 'entityref', $name); }
    public function createProcessingInstruction(string $target, string $data = "") { return __dom('create', $this->__d, 'pi', $target, $data); }
    public function createTextNode(string $data): DOMText { return __dom('create', $this->__d, 'text', $data); }
    public function getElementById(string $elementId): ?DOMElement { return __dom('byId', $this->__d, $this->__n, $elementId); }
    public function getElementsByTagName(string $qualifiedName): DOMNodeList { return __dom('byTag', $this->__d, $this->__n, $qualifiedName); }
    public function getElementsByTagNameNS(?string $namespace, string $localName): DOMNodeList { return __dom('byTagNS', $this->__d, $this->__n, $namespace, $localName); }
    public function importNode(DOMNode $node, bool $deep = false) { return __dom('importNode', $this->__d, $this->__n, $node, $deep); }
    public function adoptNode(DOMNode $node): DOMNode|false { return __dom('adoptNode', $this->__d, $this->__n, $node); }
    public function load(string $filename, int $options = 0): bool { return __dom('load', $this, $this->__d, $filename, $options, true) !== false; }
    public function loadXML(string $source, int $options = 0): bool { return __dom('load', $this, $this->__d, $source, $options, false) !== false; }
    public function normalizeDocument(): void { __dom('normalize', $this->__d, $this->__n); }
    public function registerNodeClass(string $baseClass, ?string $extendedClass): true { return true; }
    public function save(string $filename, int $options = 0): int|false { return __dom('savefile', $this->__d, 0, $filename, $options); }
    public function saveXML(?DOMNode $node = null, int $options = 0): string|false { return __dom('save', $this->__d, $node, null, $options); }
    public function loadHTML(string $source, int $options = 0): bool { return false; }
    public function loadHTMLFile(string $filename, int $options = 0): bool { return false; }
    public function saveHTML(?DOMNode $node = null): string|false { return false; }
    public function saveHTMLFile(string $filename): int|false { return false; }
    public function schemaValidate(string $filename, int $flags = 0): bool { return __dom('schemaValidate', $this->__d, 0, $filename, true, $flags); }
    public function schemaValidateSource(string $source, int $flags = 0): bool { return __dom('schemaValidate', $this->__d, 0, $source, false, $flags); }
    public function relaxNGValidate(string $filename): bool { return false; }
    public function relaxNGValidateSource(string $source): bool { return false; }
    public function validate(): bool {
        if ($this->doctype === null) { trigger_error('DOMDocument::validate(): no DTD found!', E_USER_WARNING); return false; }
        return true;
    }
    public function xinclude(int $options = 0): int|false { return __dom('xinclude', $this->__d, 0, $options); }
    public function append(...$nodes): void { foreach ($this->__nodes($nodes) as $n) { $this->appendChild($n); } }
    public function prepend(...$nodes): void { $f = $this->firstChild; foreach ($this->__nodes($nodes) as $n) { $this->insertBefore($n, $f); } }
    public function replaceChildren(...$nodes): void { while ($this->firstChild) { $this->removeChild($this->firstChild); } $this->append(...$nodes); }
}

class DOMDocumentType extends DOMNode {}
class DOMEntity extends DOMNode {}
class DOMNotation extends DOMNode {}

class DOMEntityReference extends DOMNode {
    public function __construct(string $name) { __dom('newnode', $this, 'entityref', $name); }
}

class DOMProcessingInstruction extends DOMNode {
    public function __construct(string $name, string $value = "") { __dom('newnode', $this, 'pi', $name, $value); }
}

class DOMXPath {
    public function __sleep(): array { throw new Exception("Serialization of '" . static::class . "' is not allowed"); }
    public function __wakeup(): void { throw new Exception("Unserialization of '" . static::class . "' is not allowed"); }
    public $__doc; public $__d; public $__ns = []; public $__reg = true;
    public function __construct(DOMDocument $document, bool $registerNodeNS = true) {
        $this->__doc = $document; $this->__d = $document->__d; $this->__reg = $registerNodeNS;
    }
    public function __get($name) {
        if ($name === 'document') { return $this->__doc; }
        if ($name === 'registerNodeNamespaces') { return $this->__reg; }
        return $this->$name;
    }
    public function __set($name, $value) {
        if ($name === 'registerNodeNamespaces') { $this->__reg = (bool) $value; return; }
        $this->$name = $value;
    }
    public function evaluate(string $expression, ?DOMNode $contextNode = null, bool $registerNodeNS = true): mixed {
        $reg = func_num_args() >= 3 ? $registerNodeNS : $this->__reg;
        return __dom('xpath', $this->__d, 0, $expression, $contextNode, $reg, $this->__ns, false);
    }
    public function query(string $expression, ?DOMNode $contextNode = null, bool $registerNodeNS = true): mixed {
        $reg = func_num_args() >= 3 ? $registerNodeNS : $this->__reg;
        return __dom('xpath', $this->__d, 0, $expression, $contextNode, $reg, $this->__ns, true);
    }
    public function registerNamespace(string $prefix, string $namespace): bool { $this->__ns[$prefix] = $namespace; return true; }
    public function registerPhpFunctions(string|array|null $restrict = null): void {}
    public function registerPhpFunctionNS(string $namespaceURI, string $name, callable $callable): void {}
    public static function quote(string $str): string {
        // php allocates the quoted string fallibly.
        if (!__dom('reserve', strlen($str) + 2)) { return ''; }
        if (!str_contains($str, "'")) { return "'" . $str . "'"; }
        if (!str_contains($str, '"')) { return '"' . $str . '"'; }
        // php's concat() trick: alternate quote styles per run
        $out = 'concat(';
        $len = strlen($str); $i = 0;
        while ($i < $len) {
            $sq = strpos($str, "'", $i); $dq = strpos($str, '"', $i);
            $ds = $sq === false ? $len - $i : $sq - $i;
            $dd = $dq === false ? $len - $i : $dq - $i;
            $n = max($ds, $dd);
            $q = $ds > $dd ? "'" : '"';
            $out .= $q . substr($str, $i, $n) . $q . ',';
            $i += $n;
        }
        return substr($out, 0, -1) . ')';
    }
}
