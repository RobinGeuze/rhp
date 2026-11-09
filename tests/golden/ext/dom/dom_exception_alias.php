<?php
var_dump(class_exists('Dom\DOMException'), class_exists('dom\domexception'));
try { throw new Dom\DOMException('m', 3); } catch (Dom\DOMException $e) { var_dump(get_class($e), $e->getCode()); }
function f(Dom\DOMException $e): Dom\DOMException { return $e; }
var_dump(get_class(f(new DOMException('x'))));
var_dump((new ReflectionClass('Dom\DOMException'))->getName(), new DOMException instanceof Dom\DOMException, is_a('DOMException', 'Dom\DOMException', true), is_subclass_of('DOMException', 'Exception'));
$c = get_declared_classes(); var_dump(array_values(array_filter($c, fn($x) => stripos($x, "dom") === 0 && stripos($x, "exception") !== false)));
var_dump(@class_alias('stdClass', 'Dom\DOMException')); echo error_get_last()['message'], "\n";
$d = \Dom\XMLDocument::createFromString('<root><child/></root>');
$child = $d->documentElement->firstChild; $child->remove();
try { $child->C14N(); } catch (Dom\DOMException $e) { echo $e->getMessage(), "\n"; }
