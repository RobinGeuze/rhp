<?php
// mb_output_handler(): converts to the HTTP output encoding, passes
// through with "pass", and mb_http_output() validates names.
var_dump(mb_http_output());
mb_http_output('ISO-8859-1');
ob_start('mb_output_handler');
echo "caf\u{e9}\n";
$s = ob_get_clean();
var_dump(bin2hex($s));
mb_http_output('pass');
ob_start('mb_output_handler');
echo "caf\u{e9}\n";
var_dump(bin2hex(ob_get_clean()));
var_dump(mb_http_output());
mb_http_output('HTML-ENTITIES');
var_dump(mb_output_handler("<\u{e9}>", PHP_OUTPUT_HANDLER_START | PHP_OUTPUT_HANDLER_END));
// Not started: the conversion is off again after the END call.
var_dump(mb_output_handler("\u{e9}", 0));
foreach (['BAD', "UTF-8\0x"] as $n) {
    try {
        mb_http_output($n);
    } catch (ValueError $e) {
        echo $e->getMessage(), "\n";
    }
}
var_dump(headers_list());
