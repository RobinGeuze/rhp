<?php
// Internal classes as php's stubs declare them: Exception's __clone /
// __wakeup, enum interfaces, Generator and Reflection methods, stand-in
// handlers hidden from Reflection and method_exists().
enum Suit: string { case H = 'h'; }
var_dump(method_exists('UnitEnum', 'cases'), method_exists('BackedEnum', 'tryFrom'), Suit::from('h'));
var_dump((new ReflectionClass('RoundingMode'))->isFinal(), (new ReflectionClass('Suit'))->isFinal());
var_dump((new ReflectionClassConstant('Suit', 'H'))->getModifiers(), (string) new ReflectionClassConstant('Suit', 'H'));
$m = new ReflectionMethod('Exception', '__clone'); var_dump($m->isPrivate(), method_exists('Error', '__wakeup'));
try { clone new Exception; } catch (Error $e) { echo $e->getMessage(), "\n"; }
$s = serialize(new LogicException('m', 3));
$x = unserialize(str_replace('s:1:"m";', 'a:0:{}', $s)); var_dump((new ReflectionProperty('Exception', 'message'))->isInitialized($x));
var_dump((new ReflectionProperty('ErrorException', 'severity'))->getDefaultValue(), (new ReflectionProperty('Error', 'line'))->hasDefaultValue());
$g = (function () { $x = yield 1; echo "got $x\n"; yield 2; })();
var_dump(call_user_func([$g, 'current']), (new ReflectionMethod($g, 'send'))->invoke($g, 'v'), get_class_methods($g));
var_dump(method_exists('IteratorIterator', '__call'), method_exists('DOMNode', '__get'), (new ReflectionClass('CurlHandle'))->getConstructor());
var_dump(array_map(fn ($m) => $m->name, (new ReflectionClass('ReflectionNamedType'))->getMethods()));
var_dump(array_keys((new ReflectionClass('ReflectionExtension'))->getConstants()));
$c = new ReflectionEnumBackedCase('Suit', 'H'); var_dump($c->getBackingValue());
enum U { case A; }
try { new ReflectionEnumBackedCase('U', 'A'); } catch (ReflectionException $e) { echo $e->getMessage(), "\n"; }
