<?php
// SimpleXMLElement::addChild parses entity references in its value; an
// unterminated one is a tree error whose text differs between libxml2
// releases.
set_error_handler(function ($no, $msg) { echo "E$no: $msg\n"; return true; });
$n = new SimpleXMLElement('<a/>');
var_dump($n->addChild('b', 'x&y')->getName());
$n->addChild('c', '&amp;&lt;&#65;&bogus;<');
$n->addChild('d', '&#x41;&#66;');
echo $n->asXML();
libxml_use_internal_errors(true);
$n->addChild('e', 'q&r');
foreach (libxml_get_errors() as $er) var_dump($er->level, $er->code, $er->line, $er->column, $er->message);
libxml_clear_errors();
libxml_use_internal_errors(false);
$n->e = 'a&b';
$n['at'] = '&x';
echo $n->asXML();
