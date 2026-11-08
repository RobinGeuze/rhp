<?php
// A format with an empty result: ICU's U_STRING_NOT_TERMINATED_WARNING
// does not stay in the error codes (php's UTF-8 conversion resets them).
$f = new IntlDateFormatter('en', IntlDateFormatter::FULL, IntlDateFormatter::FULL, 'UTC');
var_dump($f->format(NAN), $f->getErrorCode(), intl_get_error_code(), intl_get_error_message());
var_dump(datefmt_format($f, NAN), intl_get_error_code());
