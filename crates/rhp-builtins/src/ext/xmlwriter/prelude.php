
class XMLWriter {
    public $__w = null;
    public function openUri(string $uri): bool { if ($this->__w !== null) { __xw('free', $this->__w); } $r = __xw('openUri', 0, $uri); if ($r === false) { return false; } $this->__w = $r; return true; }
    public function openMemory(): bool { if ($this->__w !== null) { __xw('free', $this->__w); } $this->__w = __xw('openMemory', 0); return true; }
    public static function toMemory(): static { $w = new static(); $w->openMemory(); return $w; }
    public static function toUri(string $uri): static { $w = new static(); $w->openUri($uri); return $w; }
    private function __chk() { if ($this->__w === null) { throw new Error('Invalid or uninitialized XMLWriter object'); } return $this->__w; }
    public function __destruct() { if ($this->__w !== null) { __xw('flush', $this->__w, true); __xw('free', $this->__w); $this->__w = null; } }
    public function setIndent(bool $enable): bool { return __xw('setIndent', $this->__chk(), $enable); }
    public function setIndentString(string $indentation): bool { return __xw('setIndentString', $this->__chk(), $indentation); }
    public function startComment(): bool { return __xw('startComment', $this->__chk()); }
    public function endComment(): bool { return __xw('endComment', $this->__chk()); }
    public function startAttribute(string $name): bool { return __xw('startAttribute', $this->__chk(), $name); }
    public function endAttribute(): bool { return __xw('endAttribute', $this->__chk()); }
    public function writeAttribute(string $name, string $value): bool { return __xw('writeAttribute', $this->__chk(), $name, $value); }
    public function startAttributeNs(?string $prefix, string $name, ?string $namespace): bool { return __xw('startAttributeNs', $this->__chk(), $prefix, $name, $namespace); }
    public function writeAttributeNs(?string $prefix, string $name, ?string $namespace, string $value): bool { return __xw('writeAttributeNs', $this->__chk(), $prefix, $name, $namespace, $value); }
    public function startElement(string $name): bool { return __xw('startElement', $this->__chk(), $name); }
    public function endElement(): bool { return __xw('endElement', $this->__chk()); }
    public function fullEndElement(): bool { return __xw('fullEndElement', $this->__chk()); }
    public function startElementNs(?string $prefix, string $name, ?string $namespace): bool { return __xw('startElementNs', $this->__chk(), $prefix, $name, $namespace); }
    public function writeElement(string $name, ?string $content = null): bool { return __xw('writeElement', $this->__chk(), $name, $content); }
    public function writeElementNs(?string $prefix, string $name, ?string $namespace, ?string $content = null): bool { return __xw('writeElementNs', $this->__chk(), $prefix, $name, $namespace, $content); }
    public function startPi(string $target): bool { return __xw('startPi', $this->__chk(), $target); }
    public function endPi(): bool { return __xw('endPi', $this->__chk()); }
    public function writePi(string $target, string $content): bool { return __xw('writePi', $this->__chk(), $target, $content); }
    public function startCdata(): bool { return __xw('startCdata', $this->__chk()); }
    public function endCdata(): bool { return __xw('endCdata', $this->__chk()); }
    public function writeCdata(string $content): bool { return __xw('writeCdata', $this->__chk(), $content); }
    public function text(string $content): bool { return __xw('text', $this->__chk(), $content); }
    public function writeRaw(string $content): bool { return __xw('writeRaw', $this->__chk(), $content); }
    public function startDocument(?string $version = "1.0", ?string $encoding = null, ?string $standalone = null): bool { return __xw('startDocument', $this->__chk(), $version, $encoding, $standalone); }
    public function endDocument(): bool { return __xw('endDocument', $this->__chk()); }
    public function writeComment(string $content): bool { return __xw('writeComment', $this->__chk(), $content); }
    public function startDtd(string $qualifiedName, ?string $publicId = null, ?string $systemId = null): bool { return __xw('startDtd', $this->__chk(), $qualifiedName, $publicId, $systemId); }
    public function endDtd(): bool { return __xw('endDtd', $this->__chk()); }
    public function writeDtd(string $name, ?string $publicId = null, ?string $systemId = null, ?string $content = null): bool { return __xw('writeDtd', $this->__chk(), $name, $publicId, $systemId, $content); }
    public function startDtdElement(string $qualifiedName): bool { return __xw('startDtdElement', $this->__chk(), $qualifiedName); }
    public function endDtdElement(): bool { return __xw('endDtdElement', $this->__chk()); }
    public function writeDtdElement(string $name, string $content): bool { return __xw('writeDtdElement', $this->__chk(), $name, $content); }
    public function startDtdAttlist(string $name): bool { return __xw('startDtdAttlist', $this->__chk(), $name); }
    public function endDtdAttlist(): bool { return __xw('endDtdAttlist', $this->__chk()); }
    public function writeDtdAttlist(string $name, string $content): bool { return __xw('writeDtdAttlist', $this->__chk(), $name, $content); }
    public function startDtdEntity(string $name, bool $isParam): bool { return __xw('startDtdEntity', $this->__chk(), $name, $isParam); }
    public function endDtdEntity(): bool { return __xw('endDtdEntity', $this->__chk()); }
    public function writeDtdEntity(string $name, string $content, bool $isParam = false, ?string $publicId = null, ?string $systemId = null, ?string $notationData = null): bool { return __xw('writeDtdEntity', $this->__chk(), $name, $content, $isParam); }
    public function outputMemory(bool $flush = true): string { return __xw('outputMemory', $this->__chk(), $flush); }
    public function flush(bool $empty = true): string|int { return __xw('flush', $this->__chk(), $empty); }
}
function xmlwriter_open_uri(string $uri): XMLWriter|false { $w = new XMLWriter(); return $w->openUri($uri) ? $w : false; }
function xmlwriter_open_memory(): XMLWriter|false { $w = new XMLWriter(); $w->openMemory(); return $w; }
function xmlwriter_set_indent(XMLWriter $writer, bool $enable): bool { return $writer->setIndent($enable); }
function xmlwriter_set_indent_string(XMLWriter $writer, string $indentation): bool { return $writer->setIndentString($indentation); }
function xmlwriter_start_comment(XMLWriter $writer): bool { return $writer->startComment(); }
function xmlwriter_end_comment(XMLWriter $writer): bool { return $writer->endComment(); }
function xmlwriter_start_attribute(XMLWriter $writer, string $name): bool { return $writer->startAttribute($name); }
function xmlwriter_end_attribute(XMLWriter $writer): bool { return $writer->endAttribute(); }
function xmlwriter_write_attribute(XMLWriter $writer, string $name, string $value): bool { return $writer->writeAttribute($name, $value); }
function xmlwriter_start_attribute_ns(XMLWriter $writer, ?string $prefix, string $name, ?string $namespace): bool { return $writer->startAttributeNs($prefix, $name, $namespace); }
function xmlwriter_write_attribute_ns(XMLWriter $writer, ?string $prefix, string $name, ?string $namespace, string $value): bool { return $writer->writeAttributeNs($prefix, $name, $namespace, $value); }
function xmlwriter_start_element(XMLWriter $writer, string $name): bool { return $writer->startElement($name); }
function xmlwriter_end_element(XMLWriter $writer): bool { return $writer->endElement(); }
function xmlwriter_full_end_element(XMLWriter $writer): bool { return $writer->fullEndElement(); }
function xmlwriter_start_element_ns(XMLWriter $writer, ?string $prefix, string $name, ?string $namespace): bool { return $writer->startElementNs($prefix, $name, $namespace); }
function xmlwriter_write_element(XMLWriter $writer, string $name, ?string $content = null): bool { return $writer->writeElement($name, $content); }
function xmlwriter_write_element_ns(XMLWriter $writer, ?string $prefix, string $name, ?string $namespace, ?string $content = null): bool { return $writer->writeElementNs($prefix, $name, $namespace, $content); }
function xmlwriter_start_pi(XMLWriter $writer, string $target): bool { return $writer->startPi($target); }
function xmlwriter_end_pi(XMLWriter $writer): bool { return $writer->endPi(); }
function xmlwriter_write_pi(XMLWriter $writer, string $target, string $content): bool { return $writer->writePi($target, $content); }
function xmlwriter_start_cdata(XMLWriter $writer): bool { return $writer->startCdata(); }
function xmlwriter_end_cdata(XMLWriter $writer): bool { return $writer->endCdata(); }
function xmlwriter_write_cdata(XMLWriter $writer, string $content): bool { return $writer->writeCdata($content); }
function xmlwriter_text(XMLWriter $writer, string $content): bool { return $writer->text($content); }
function xmlwriter_write_raw(XMLWriter $writer, string $content): bool { return $writer->writeRaw($content); }
function xmlwriter_start_document(XMLWriter $writer, ?string $version = "1.0", ?string $encoding = null, ?string $standalone = null): bool { return $writer->startDocument($version, $encoding, $standalone); }
function xmlwriter_end_document(XMLWriter $writer): bool { return $writer->endDocument(); }
function xmlwriter_write_comment(XMLWriter $writer, string $content): bool { return $writer->writeComment($content); }
function xmlwriter_start_dtd(XMLWriter $writer, string $qualifiedName, ?string $publicId = null, ?string $systemId = null): bool { return $writer->startDtd($qualifiedName, $publicId, $systemId); }
function xmlwriter_end_dtd(XMLWriter $writer): bool { return $writer->endDtd(); }
function xmlwriter_write_dtd(XMLWriter $writer, string $name, ?string $publicId = null, ?string $systemId = null, ?string $content = null): bool { return $writer->writeDtd($name, $publicId, $systemId, $content); }
function xmlwriter_start_dtd_element(XMLWriter $writer, string $qualifiedName): bool { return $writer->startDtdElement($qualifiedName); }
function xmlwriter_end_dtd_element(XMLWriter $writer): bool { return $writer->endDtdElement(); }
function xmlwriter_write_dtd_element(XMLWriter $writer, string $name, string $content): bool { return $writer->writeDtdElement($name, $content); }
function xmlwriter_start_dtd_attlist(XMLWriter $writer, string $name): bool { return $writer->startDtdAttlist($name); }
function xmlwriter_end_dtd_attlist(XMLWriter $writer): bool { return $writer->endDtdAttlist(); }
function xmlwriter_write_dtd_attlist(XMLWriter $writer, string $name, string $content): bool { return $writer->writeDtdAttlist($name, $content); }
function xmlwriter_start_dtd_entity(XMLWriter $writer, string $name, bool $isParam): bool { return $writer->startDtdEntity($name, $isParam); }
function xmlwriter_end_dtd_entity(XMLWriter $writer): bool { return $writer->endDtdEntity(); }
function xmlwriter_write_dtd_entity(XMLWriter $writer, string $name, string $content, bool $isParam = false, ?string $publicId = null, ?string $systemId = null, ?string $notationData = null): bool { return $writer->writeDtdEntity($name, $content, $isParam, $publicId, $systemId, $notationData); }
function xmlwriter_output_memory(XMLWriter $writer, bool $flush = true): string { return $writer->outputMemory($flush); }
function xmlwriter_flush(XMLWriter $writer, bool $empty = true): string|int { return $writer->flush($empty); }
