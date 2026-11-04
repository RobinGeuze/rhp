
function utf8_encode(string $string): string {
    __stdext_deprecated('Function utf8_encode() is deprecated since 8.2, visit the php.net documentation for various alternatives');
    return mb_convert_encoding($string, 'UTF-8', 'ISO-8859-1');
}
function utf8_decode(string $string): string {
    __stdext_deprecated('Function utf8_decode() is deprecated since 8.2, visit the php.net documentation for various alternatives');
    return mb_convert_encoding($string, 'ISO-8859-1', 'UTF-8');
}
