<?php
// Dom\HTMLDocument::saveHtmlFile writes each serializer piece to the
// stream as it goes (dom_common_save), in the document's encoding.
class LogWrapper {
    public $context;
    public static array $writes = [];
    public function stream_open($path, $mode, $options, &$opened_path) { return true; }
    public function stream_write(string $data): int { self::$writes[] = $data; return strlen($data); }
    public function stream_eof() { return false; }
    public function stream_close() { return true; }
    public function stream_tell() { return 0; }
}
stream_wrapper_register("log", "LogWrapper");

$dom = Dom\HTMLDocument::createFromString('<!DOCTYPE html><p class="a&quot;b" data-x=1>caf&eacute; &amp; <b>x</b></p><!--c--><script>1<2</script>', LIBXML_NOERROR);
var_dump($dom->saveHtmlFile("log://a"));
echo implode('|', LogWrapper::$writes), "\n";
LogWrapper::$writes = [];
$dom->charset = 'windows-1252';
var_dump($dom->saveHtmlFile("log://b"));
echo bin2hex(implode('|', LogWrapper::$writes)), "\n";
var_dump($dom->saveHtml() === implode('', LogWrapper::$writes));

$f = __DIR__ . '/dom_html_save_chunks.tmp';
var_dump($dom->saveHtmlFile($f), filesize($f));
unlink($f);
var_dump(@$dom->saveHtmlFile(__DIR__ . '/no/such/dir/x.html'));
