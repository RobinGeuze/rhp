class XMLWriter {
    public function openUri(string $uri): bool { return __xmlwriter('openUri', $this, $uri); }
    public static function toUri(string $uri): static { return __xmlwriter_to('toUri', static::class, $uri); }
    public function openMemory(): bool { return __xmlwriter('openMemory', $this); }
    public static function toMemory(): static { return __xmlwriter_to('toMemory', static::class); }
    public static function toStream($stream): static { return __xmlwriter_to('toStream', static::class, $stream); }
    public function setIndent(bool $enable): bool { return __xmlwriter('setIndent', $this, $enable); }
    public function setIndentString(string $indentation): bool { return __xmlwriter('setIndentString', $this, $indentation); }
    public function startComment(): bool { return __xmlwriter('startComment', $this); }
    public function endComment(): bool { return __xmlwriter('endComment', $this); }
    public function startAttribute(string $name): bool { return __xmlwriter('startAttribute', $this, $name); }
    public function endAttribute(): bool { return __xmlwriter('endAttribute', $this); }
    public function writeAttribute(string $name, string $value): bool { return __xmlwriter('writeAttribute', $this, $name, $value); }
    public function startAttributeNs(?string $prefix, string $name, ?string $namespace): bool { return __xmlwriter('startAttributeNs', $this, $prefix, $name, $namespace); }
    public function writeAttributeNs(?string $prefix, string $name, ?string $namespace, string $value): bool { return __xmlwriter('writeAttributeNs', $this, $prefix, $name, $namespace, $value); }
    public function startElement(string $name): bool { return __xmlwriter('startElement', $this, $name); }
    public function endElement(): bool { return __xmlwriter('endElement', $this); }
    public function fullEndElement(): bool { return __xmlwriter('fullEndElement', $this); }
    public function startElementNs(?string $prefix, string $name, ?string $namespace): bool { return __xmlwriter('startElementNs', $this, $prefix, $name, $namespace); }
    public function writeElement(string $name, ?string $content = null): bool { return __xmlwriter('writeElement', $this, $name, $content); }
    public function writeElementNs(?string $prefix, string $name, ?string $namespace, ?string $content = null): bool { return __xmlwriter('writeElementNs', $this, $prefix, $name, $namespace, $content); }
    public function startPi(string $target): bool { return __xmlwriter('startPi', $this, $target); }
    public function endPi(): bool { return __xmlwriter('endPi', $this); }
    public function writePi(string $target, string $content): bool { return __xmlwriter('writePi', $this, $target, $content); }
    public function startCdata(): bool { return __xmlwriter('startCdata', $this); }
    public function endCdata(): bool { return __xmlwriter('endCdata', $this); }
    public function writeCdata(string $content): bool { return __xmlwriter('writeCdata', $this, $content); }
    public function text(string $content): bool { return __xmlwriter('text', $this, $content); }
    public function writeRaw(string $content): bool { return __xmlwriter('writeRaw', $this, $content); }
    public function startDocument(?string $version = "1.0", ?string $encoding = null, ?string $standalone = null): bool { return __xmlwriter('startDocument', $this, $version, $encoding, $standalone); }
    public function endDocument(): bool { return __xmlwriter('endDocument', $this); }
    public function writeComment(string $content): bool { return __xmlwriter('writeComment', $this, $content); }
    public function startDtd(string $qualifiedName, ?string $publicId = null, ?string $systemId = null): bool { return __xmlwriter('startDtd', $this, $qualifiedName, $publicId, $systemId); }
    public function endDtd(): bool { return __xmlwriter('endDtd', $this); }
    public function writeDtd(string $name, ?string $publicId = null, ?string $systemId = null, ?string $content = null): bool { return __xmlwriter('writeDtd', $this, $name, $publicId, $systemId, $content); }
    public function startDtdElement(string $qualifiedName): bool { return __xmlwriter('startDtdElement', $this, $qualifiedName); }
    public function endDtdElement(): bool { return __xmlwriter('endDtdElement', $this); }
    public function writeDtdElement(string $name, string $content): bool { return __xmlwriter('writeDtdElement', $this, $name, $content); }
    public function startDtdAttlist(string $name): bool { return __xmlwriter('startDtdAttlist', $this, $name); }
    public function endDtdAttlist(): bool { return __xmlwriter('endDtdAttlist', $this); }
    public function writeDtdAttlist(string $name, string $content): bool { return __xmlwriter('writeDtdAttlist', $this, $name, $content); }
    public function startDtdEntity(string $name, bool $isParam): bool { return __xmlwriter('startDtdEntity', $this, $name, $isParam); }
    public function endDtdEntity(): bool { return __xmlwriter('endDtdEntity', $this); }
    public function writeDtdEntity(string $name, string $content, bool $isParam = false, ?string $publicId = null, ?string $systemId = null, ?string $notationData = null): bool { return __xmlwriter('writeDtdEntity', $this, $name, $content, $isParam, $publicId, $systemId, $notationData); }
    public function outputMemory(bool $flush = true): string { return __xmlwriter('outputMemory', $this, $flush); }
    public function flush(bool $empty = true): string|int { return __xmlwriter('flush', $this, $empty); }
}
function xmlwriter_open_uri(string $uri): XMLWriter|false { return __xmlwriter_fn('xmlwriter_open_uri', $uri); }
function xmlwriter_open_memory(): XMLWriter|false { return __xmlwriter_fn('xmlwriter_open_memory'); }
function xmlwriter_set_indent(XMLWriter $writer, bool $enable): bool { return __xmlwriter_fn('xmlwriter_set_indent', $writer, $enable); }
function xmlwriter_set_indent_string(XMLWriter $writer, string $indentation): bool { return __xmlwriter_fn('xmlwriter_set_indent_string', $writer, $indentation); }
function xmlwriter_start_comment(XMLWriter $writer): bool { return __xmlwriter_fn('xmlwriter_start_comment', $writer); }
function xmlwriter_end_comment(XMLWriter $writer): bool { return __xmlwriter_fn('xmlwriter_end_comment', $writer); }
function xmlwriter_start_attribute(XMLWriter $writer, string $name): bool { return __xmlwriter_fn('xmlwriter_start_attribute', $writer, $name); }
function xmlwriter_end_attribute(XMLWriter $writer): bool { return __xmlwriter_fn('xmlwriter_end_attribute', $writer); }
function xmlwriter_write_attribute(XMLWriter $writer, string $name, string $value): bool { return __xmlwriter_fn('xmlwriter_write_attribute', $writer, $name, $value); }
function xmlwriter_start_attribute_ns(XMLWriter $writer, ?string $prefix, string $name, ?string $namespace): bool { return __xmlwriter_fn('xmlwriter_start_attribute_ns', $writer, $prefix, $name, $namespace); }
function xmlwriter_write_attribute_ns(XMLWriter $writer, ?string $prefix, string $name, ?string $namespace, string $value): bool { return __xmlwriter_fn('xmlwriter_write_attribute_ns', $writer, $prefix, $name, $namespace, $value); }
function xmlwriter_start_element(XMLWriter $writer, string $name): bool { return __xmlwriter_fn('xmlwriter_start_element', $writer, $name); }
function xmlwriter_end_element(XMLWriter $writer): bool { return __xmlwriter_fn('xmlwriter_end_element', $writer); }
function xmlwriter_full_end_element(XMLWriter $writer): bool { return __xmlwriter_fn('xmlwriter_full_end_element', $writer); }
function xmlwriter_start_element_ns(XMLWriter $writer, ?string $prefix, string $name, ?string $namespace): bool { return __xmlwriter_fn('xmlwriter_start_element_ns', $writer, $prefix, $name, $namespace); }
function xmlwriter_write_element(XMLWriter $writer, string $name, ?string $content = null): bool { return __xmlwriter_fn('xmlwriter_write_element', $writer, $name, $content); }
function xmlwriter_write_element_ns(XMLWriter $writer, ?string $prefix, string $name, ?string $namespace, ?string $content = null): bool { return __xmlwriter_fn('xmlwriter_write_element_ns', $writer, $prefix, $name, $namespace, $content); }
function xmlwriter_start_pi(XMLWriter $writer, string $target): bool { return __xmlwriter_fn('xmlwriter_start_pi', $writer, $target); }
function xmlwriter_end_pi(XMLWriter $writer): bool { return __xmlwriter_fn('xmlwriter_end_pi', $writer); }
function xmlwriter_write_pi(XMLWriter $writer, string $target, string $content): bool { return __xmlwriter_fn('xmlwriter_write_pi', $writer, $target, $content); }
function xmlwriter_start_cdata(XMLWriter $writer): bool { return __xmlwriter_fn('xmlwriter_start_cdata', $writer); }
function xmlwriter_end_cdata(XMLWriter $writer): bool { return __xmlwriter_fn('xmlwriter_end_cdata', $writer); }
function xmlwriter_write_cdata(XMLWriter $writer, string $content): bool { return __xmlwriter_fn('xmlwriter_write_cdata', $writer, $content); }
function xmlwriter_text(XMLWriter $writer, string $content): bool { return __xmlwriter_fn('xmlwriter_text', $writer, $content); }
function xmlwriter_write_raw(XMLWriter $writer, string $content): bool { return __xmlwriter_fn('xmlwriter_write_raw', $writer, $content); }
function xmlwriter_start_document(XMLWriter $writer, ?string $version = "1.0", ?string $encoding = null, ?string $standalone = null): bool { return __xmlwriter_fn('xmlwriter_start_document', $writer, $version, $encoding, $standalone); }
function xmlwriter_end_document(XMLWriter $writer): bool { return __xmlwriter_fn('xmlwriter_end_document', $writer); }
function xmlwriter_write_comment(XMLWriter $writer, string $content): bool { return __xmlwriter_fn('xmlwriter_write_comment', $writer, $content); }
function xmlwriter_start_dtd(XMLWriter $writer, string $qualifiedName, ?string $publicId = null, ?string $systemId = null): bool { return __xmlwriter_fn('xmlwriter_start_dtd', $writer, $qualifiedName, $publicId, $systemId); }
function xmlwriter_end_dtd(XMLWriter $writer): bool { return __xmlwriter_fn('xmlwriter_end_dtd', $writer); }
function xmlwriter_write_dtd(XMLWriter $writer, string $name, ?string $publicId = null, ?string $systemId = null, ?string $content = null): bool { return __xmlwriter_fn('xmlwriter_write_dtd', $writer, $name, $publicId, $systemId, $content); }
function xmlwriter_start_dtd_element(XMLWriter $writer, string $qualifiedName): bool { return __xmlwriter_fn('xmlwriter_start_dtd_element', $writer, $qualifiedName); }
function xmlwriter_end_dtd_element(XMLWriter $writer): bool { return __xmlwriter_fn('xmlwriter_end_dtd_element', $writer); }
function xmlwriter_write_dtd_element(XMLWriter $writer, string $name, string $content): bool { return __xmlwriter_fn('xmlwriter_write_dtd_element', $writer, $name, $content); }
function xmlwriter_start_dtd_attlist(XMLWriter $writer, string $name): bool { return __xmlwriter_fn('xmlwriter_start_dtd_attlist', $writer, $name); }
function xmlwriter_end_dtd_attlist(XMLWriter $writer): bool { return __xmlwriter_fn('xmlwriter_end_dtd_attlist', $writer); }
function xmlwriter_write_dtd_attlist(XMLWriter $writer, string $name, string $content): bool { return __xmlwriter_fn('xmlwriter_write_dtd_attlist', $writer, $name, $content); }
function xmlwriter_start_dtd_entity(XMLWriter $writer, string $name, bool $isParam): bool { return __xmlwriter_fn('xmlwriter_start_dtd_entity', $writer, $name, $isParam); }
function xmlwriter_end_dtd_entity(XMLWriter $writer): bool { return __xmlwriter_fn('xmlwriter_end_dtd_entity', $writer); }
function xmlwriter_write_dtd_entity(XMLWriter $writer, string $name, string $content, bool $isParam = false, ?string $publicId = null, ?string $systemId = null, ?string $notationData = null): bool { return __xmlwriter_fn('xmlwriter_write_dtd_entity', $writer, $name, $content, $isParam, $publicId, $systemId, $notationData); }
function xmlwriter_output_memory(XMLWriter $writer, bool $flush = true): string { return __xmlwriter_fn('xmlwriter_output_memory', $writer, $flush); }
function xmlwriter_flush(XMLWriter $writer, bool $empty = true): string|int { return __xmlwriter_fn('xmlwriter_flush', $writer, $empty); }
