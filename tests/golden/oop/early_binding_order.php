<?php
// Which top-level classes php early-binds (zend_compile_class_decl): only
// those without interfaces (an implicit Stringable counts), traits or enum
// kind, and whose parent is already declared — in declaration order.
require __DIR__ . '/early_binding_order.inc';
echo get_parent_class('EarlyBind\TokenPoly'), "\n";

var_dump(
    class_exists('EB1', false),        // plain: hoisted
    class_exists('EB2', false),        // parent EB1 declared just above it: hoisted
    class_exists('EB3', false),        // parent declared later: not hoisted
    interface_exists('EBI1', false),   // interface without parents: hoisted
    interface_exists('EBI2', false),   // interface extending one: not hoisted
    class_exists('EBT1', false),       // traits are not classes
    enum_exists('EBE1', false),        // enums: not hoisted
    class_exists('EB4', false),        // implements: not hoisted
    class_exists('EB5', false),        // uses a trait: not hoisted
    class_exists('EB6', false),        // __toString (Stringable): not hoisted
    class_exists('EB7', false),        // extends a non-hoisted class: not hoisted
);
class EB1 {}
class EB2 extends EB1 {}
class EB3 extends EB9 {}
class EB9 {}
interface EBI1 {}
trait EBT1 {}
interface EBI2 extends EBI1 {}
enum EBE1 {}
class EB4 implements EBI1 {}
class EB5 { use EBT1; }
class EB6 { public function __toString(): string { return 'six'; } }
class EB7 extends EB4 {}
// After execution reached them, all are declared.
var_dump(class_exists('EB3', false), class_exists('EB4', false), class_exists('EB6', false), class_exists('EB7', false));
var_dump((string) new EB6, new EB7 instanceof EBI1, EBE1::class);
// An anonymous class implementing an interface still works.
$anon = new class implements EBI1 { public function __toString(): string { return 'anon'; } };
var_dump($anon instanceof EBI1, (string) $anon);
