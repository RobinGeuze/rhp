<?php
namespace App\Model;

use Attribute;

#[Attribute(Attribute::TARGET_CLASS | Attribute::TARGET_METHOD | Attribute::IS_REPEATABLE)]
class Tag {
    public function __construct(public string $name, public array $opts = []) {}
}

#[Attribute(Attribute::TARGET_PROPERTY | Attribute::TARGET_PARAMETER)]
final class Inject {
    public function __construct(public ?string $id = null) {}
}

interface HasName {
    /** Returns the name. */
    public function getName(): string;
}

trait Greets {
    public string $greeting = 'hi';
    public function greet(): string { return $this->greeting; }
}

/**
 * A user.
 */
#[Tag('entity', opts: ['table' => 'users'])]
#[Tag(name: 'second')]
abstract class Base implements HasName {
    public const VERSION = 2;
    /** the id */
    protected int $id = 0;
    private static array $registry = [];
    abstract protected function kind(): string;
    public function getName(): string { return static::class; }
}

final class User extends Base implements \Countable {
    use Greets;
    final public const ROLE = 'user';
    #[Inject('db')]
    public ?\stdClass $db = null;
    public readonly string $email;

    public function __construct(
        #[Inject] private int|string $key,
        string $email = 'x@y.z',
        public bool $active = true,
        int ...$rest
    ) {
        $this->email = $email;
    }

    /**
     * Counts.
     */
    #[Tag('m')]
    public function count(): int { return 0; }
    protected function kind(): string { return 'user'; }
    public static function make(self $other, ?callable $cb = null, (\Iterator&\Countable)|null $both = null): static { return $other; }
    private function &secret(array|null $a = [1, 2], float $f = M_PI, $c = self::ROLE): ?array { static $x; return $a; }
}

enum Suit: string {
    case Hearts = 'H';
    /** spades doc */
    case Spades = 'S';
    const Wild = self::Spades;
    public function label(): string { return ucfirst($this->name); }
}

function helper(int $a, string ...$more): void {}

$rc = new \ReflectionClass(User::class);
var_dump($rc->getName(), $rc->getShortName(), $rc->getNamespaceName(), $rc->inNamespace());
var_dump($rc->isFinal(), $rc->isAbstract(), $rc->isInterface(), $rc->isInstantiable(), $rc->isCloneable(), $rc->isInternal(), $rc->isUserDefined());
var_dump(basename($rc->getFileName()), $rc->getStartLine(), $rc->getEndLine(), $rc->getDocComment(), $rc->getModifiers());
var_dump($rc->getParentClass()->getName(), $rc->getInterfaceNames(), $rc->getTraitNames(), array_keys($rc->getTraits()));
var_dump(array_map(fn($m) => $m->class . '::' . $m->name, $rc->getMethods()));
var_dump(array_map(fn($m) => $m->name, $rc->getMethods(\ReflectionMethod::IS_STATIC | \ReflectionMethod::IS_PRIVATE)));
var_dump(array_map(fn($p) => $p->class . '::' . $p->name, $rc->getProperties()));
var_dump(array_map(fn($p) => $p->name, $rc->getProperties(\ReflectionProperty::IS_PUBLIC)));
var_dump($rc->getConstants(), $rc->hasMethod('COUNT'), $rc->hasProperty('id'), $rc->hasConstant('VERSION'));
var_dump($rc->implementsInterface('Countable'), $rc->isSubclassOf(Base::class), $rc->getConstructor()->getNumberOfParameters(), $rc->getConstructor()->getNumberOfRequiredParameters());

$base = new \ReflectionClass(Base::class);
var_dump($base->getDocComment(), $base->getModifiers(), $base->isAbstract(), $base->isInstantiable());
foreach ($base->getAttributes() as $a) {
    var_dump($a->getName(), $a->getArguments(), $a->getTarget(), $a->isRepeated(), $a->newInstance()->name);
}
var_dump(count($base->getAttributes(Tag::class)), count($base->getAttributes('app\model\tag')), count($base->getAttributes(Inject::class)));
var_dump(count($base->getAttributes(\stdClass::class, \ReflectionAttribute::IS_INSTANCEOF)));

$ctor = $rc->getConstructor();
foreach ($ctor->getParameters() as $p) {
    var_dump($p->getName(), $p->getPosition(), (string) $p->getType(), $p->allowsNull(), $p->isOptional(), $p->isVariadic(), $p->isPromoted(), $p->isDefaultValueAvailable());
    if ($p->isDefaultValueAvailable()) var_dump($p->getDefaultValue());
    foreach ($p->getAttributes() as $a) var_dump($a->getName(), $a->getTarget(), $a->newInstance()->id);
}
$t = $ctor->getParameters()[0]->getType();
var_dump(get_class($t), array_map(fn($x) => $x->getName(), $t->getTypes()));

$m = new \ReflectionMethod(User::class, 'secret');
var_dump($m->isPrivate(), $m->returnsReference(), (string) $m->getReturnType(), $m->getReturnType()->allowsNull(), \Reflection::getModifierNames($m->getModifiers()));
foreach ($m->getParameters() as $p) {
    var_dump((string) $p->getType(), $p->getDefaultValue(), $p->isDefaultValueConstant());
}
$mk = \ReflectionMethod::createFromMethodName(User::class . '::make');
var_dump($mk->isStatic(), (string) $mk->getReturnType());
foreach ($mk->getParameters() as $p) { $ty = $p->getType(); var_dump(get_class($ty), (string) $ty, $ty->allowsNull()); }
$cnt = $rc->getMethod('count');
var_dump($cnt->getDocComment(), $cnt->getStartLine(), $cnt->getEndLine(), $cnt->getAttributes()[0]->getArguments(), $cnt->getPrototype()->class);
$gn = new \ReflectionMethod(HasName::class, 'getName');
var_dump($gn->isAbstract(), $gn->getDocComment(), $gn->class);

$p = new \ReflectionProperty(User::class, 'db');
var_dump($p->isPublic(), (string) $p->getType(), $p->hasDefaultValue(), $p->getDefaultValue(), $p->getAttributes()[0]->newInstance()->id, $p->getDocComment());
$p = new \ReflectionProperty(User::class, 'email');
var_dump($p->isReadOnly(), $p->getModifiers(), $p->hasDefaultValue(), $p->isPromoted());
$p = new \ReflectionProperty(User::class, 'key');
var_dump($p->isPrivate(), $p->isPromoted(), (string) $p->getType(), $p->getAttributes()[0]->getName());
$p = new \ReflectionProperty(Base::class, 'id');
var_dump($p->getDocComment(), $p->isProtected(), $p->getDefaultValue());
$u = new User(5, 'a@b.c');
var_dump($p->getValue($u));
$p->setValue($u, 42);
var_dump($p->getValue($u), (new \ReflectionProperty(User::class, 'email'))->isInitialized($u));

$c = new \ReflectionClassConstant(User::class, 'ROLE');
var_dump($c->getValue(), $c->isFinal(), $c->isPublic(), $c->getDeclaringClass()->getName(), $c->getModifiers());
$re = new \ReflectionEnum(Suit::class);
var_dump($re->isBacked(), (string) $re->getBackingType(), count($re->getCases()), $re->getCase('Spades')->getBackingValue(), $re->getCase('Spades')->getDocComment(), $re->hasCase('Wild'));
var_dump($re->getConstants());

$f = new \ReflectionFunction('App\Model\helper');
var_dump($f->getName(), $f->getShortName(), $f->getNamespaceName(), $f->isVariadic(), (string) $f->getReturnType(), $f->getNumberOfParameters());
$cl = function (int $x) use ($u): int { return $x; };
$rf = new \ReflectionFunction($cl);
var_dump($rf->isClosure(), array_keys($rf->getClosureUsedVariables()), $rf->getClosureScopeClass(), $rf->invoke(3));
var_dump((new \ReflectionFunction('strlen'))->isInternal(), (new \ReflectionClass('Exception'))->isInternal(), (new \ReflectionClass('ArrayAccess'))->isInterface());
try { new \ReflectionMethod(User::class, 'nope'); } catch (\ReflectionException $e) { var_dump($e->getMessage()); }
try { new \ReflectionProperty(User::class, 'nope'); } catch (\ReflectionException $e) { var_dump($e->getMessage()); }
try { new \ReflectionClass('Nope\Nope'); } catch (\ReflectionException $e) { var_dump($e->getMessage()); }
try { new \ReflectionFunction('nope'); } catch (\ReflectionException $e) { var_dump($e->getMessage()); }
try { $base->getAttributes(null, 5); } catch (\ValueError $e) { var_dump($e->getMessage()); }
var_dump(\ReflectionMethod::IS_PUBLIC, \ReflectionProperty::IS_READONLY, \ReflectionClass::IS_FINAL, \Attribute::TARGET_ALL);
var_dump($rc->getDefaultProperties(), $rc->getStaticProperties());
var_dump($rc->newInstanceArgs([1, 'q@q.q'])->email, $rc->newInstanceWithoutConstructor() instanceof User);
