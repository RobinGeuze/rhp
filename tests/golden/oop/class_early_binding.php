<?php
// php early-binds a top-level class at compile time only when it implements
// no interfaces (an implicit Stringable from __toString, UnitEnum for an
// enum) and uses no traits; the others exist once their declaration runs.
var_dump(class_exists('Plain', false), class_exists('WithIface', false),
    class_exists('WithToString', false), class_exists('WithTrait', false),
    enum_exists('En', false), interface_exists('J', false),
    interface_exists('K', false), class_exists('Child', false));

interface J {}
interface K extends J {}
class Plain {}
class WithIface implements J {}
class WithToString { function __toString(): string { return 's'; } }
trait Tr {}
class WithTrait { use Tr; }
enum En { case A; }
abstract class Base {}
class Child extends Base {}

var_dump(class_exists('WithIface', false), class_exists('WithToString', false));

// Early binding is one pass in source order: a child of a class declared
// further down, or of a class that is not early-bound, waits for its
// declaration statement; a chain declared parent-first binds entirely.
var_dump(class_exists('Grand', false), class_exists('ChildOfLater', false),
    class_exists('ChildOfIface', false), class_exists('FromInternal', false));
class Mid extends Plain {}
class Grand extends Mid {}
class ChildOfLater extends Later {}
class Later {}
class ChildOfIface extends WithIface {}
class FromInternal extends Exception {}
var_dump(class_exists('ChildOfLater', false), class_exists('ChildOfIface', false));

// A conditional declaration followed by an early `return` and a fallback
// definition of the same (non-early-bindable) class: the polyfill pattern.
$f = sys_get_temp_dir() . '/rhp_early_bind_' . getmypid() . '.php';
file_put_contents($f, '<?php
namespace Poly;
if (\PHP_VERSION_ID >= 80000) {
    class Tok extends \ArrayObject {}
    return;
}
class Tok { public function __toString(): string { return ""; } }
');
require $f;
unlink($f);
var_dump(get_parent_class('Poly\Tok'));
