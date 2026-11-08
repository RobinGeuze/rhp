<?php
namespace Big;
use PHPUnit\Framework\TestCase;
use PHPUnit\Framework\Attributes\DataProvider;
use PHPUnit\Framework\Attributes\RunInSeparateProcess;
use PHPUnit\Framework\Attributes\PreserveGlobalState;
use PHPUnit\Framework\Attributes\Depends;
use PHPUnit\Framework\Attributes\TestWith;

interface Repo { public function find(int $id): ?array; public function save(array $row): int; }
abstract class Base { abstract public function name(): string; public function greet(): string { return 'hi ' . $this->name(); } }
final class Value implements \JsonSerializable { public function __construct(public readonly int $a, public array $b = []) {} public function jsonSerialize(): mixed { return ['a'=>$this->a]; } }

class MockTest extends TestCase
{
    public function testMockExpectations(): void {
        $r = $this->createMock(Repo::class);
        $r->expects($this->once())->method('find')->with(3)->willReturn(['id'=>3]);
        $r->expects($this->exactly(2))->method('save')->willReturnOnConsecutiveCalls(1, 2);
        $this->assertSame(['id'=>3], $r->find(3));
        $this->assertSame(1, $r->save([])); $this->assertSame(2, $r->save([]));
    }
    public function testStubCallback(): void {
        $r = $this->createStub(Repo::class);
        $r->method('find')->willReturnCallback(fn(int $i) => $i > 5 ? null : ['id'=>$i]);
        $this->assertNull($r->find(9)); $this->assertSame(['id'=>1], $r->find(1));
    }
    public function testPartialAbstract(): void {
        $m = $this->getMockBuilder(Base::class)->onlyMethods(['name'])->getMock();
        $m->method('name')->willReturn('bob');
        $this->assertSame('hi bob', $m->greet());
    }
    public function testMockFailsUnmet(): void {
        $r = $this->createMock(Repo::class);
        $r->expects($this->once())->method('find');
    }
    public function testArrayDiff(): void {
        $this->assertEquals(['a'=>1,'b'=>[1,2,3],'c'=>'x'], ['a'=>1,'b'=>[1,2,4],'c'=>'y']);
    }
    public function testObjectDiff(): void {
        $this->assertEquals(new Value(1, [1.5, 'q']), new Value(1, [1.25, 'q']));
    }
    public function testStringDiff(): void {
        $this->assertSame("line1\nline2\nline3\n", "line1\nlineX\nline3\n");
    }
    public function testFloatDelta(): void { $this->assertEqualsWithDelta(0.3, 0.1+0.2, 1e-9); $this->assertSame(0.30000000000000004, 0.1+0.2); }
    public function testJson(): void { $this->assertJsonStringEqualsJsonString('{"a":1,"b":[1,2]}', json_encode(['b'=>[1,3],'a'=>1])); }
    public function testException(): void { $this->expectException(\DomainException::class); $this->expectExceptionMessageMatches('/^bad \d+$/'); throw new \DomainException('bad 42'); }
    public function testOutput(): void { $this->expectOutputString("hello\n"); echo "hello\n"; }
    public function testWarning(): void { $x = []; $this->assertNull(@$x['nope']); $y = $x['missing'] ?? 1; $this->assertSame(1, $y); trigger_error('custom warning', E_USER_WARNING); }
    public function testDeprecation(): void { trigger_error('old api', E_USER_DEPRECATED); $this->assertTrue(true); }
    public function testNotice(): void { $a = []; $b = $a['undef']; $this->assertNull($b); }
    public function testRisky(): void { }
    public function testIncomplete(): void { $this->markTestIncomplete('later'); }
    #[RunInSeparateProcess]
    public function testIsolated(): void { define('ISOLATED_CONST', getmypid()); $this->assertTrue(defined('ISOLATED_CONST')); $GLOBALS['iso'] = 1; }
    #[RunInSeparateProcess]
    #[PreserveGlobalState(false)]
    public function testIsolatedFails(): void { $this->assertSame(1, 2); }
    public function testNotIsolated(): void { $this->assertFalse(defined('ISOLATED_CONST')); }
    public function testProducer(): int { $this->assertTrue(true); return 5; }
    #[Depends('testProducer')]
    public function testConsumer(int $v): void { $this->assertSame(5, $v); }
    public static function bigProvider(): iterable { for ($i = 0; $i < 300; $i++) { yield "case $i" => [$i, $i * $i, str_repeat('x', $i % 7)]; } }
    #[DataProvider('bigProvider')]
    public function testSquares(int $i, int $sq, string $s): void { $this->assertSame($sq, $i ** 2); $this->assertSame($i % 7, strlen($s)); if ($i === 137) { $this->assertSame('nope', $s); } }
    #[TestWith([1, 2, 3])]
    #[TestWith([2, 2, 5])]
    public function testWith(int $a, int $b, int $c): void { $this->assertSame($c, $a + $b); }
    public function testErrorThrown(): void { $o = null; $o->foo(); }
    public function testTypeError(): void { strlen([]); }
    public function testSprintfFormats(): void { $this->assertSame('1.50|0x1f|  ab|1.0e+3', sprintf('%.2f|0x%x|%4s|%.1e', 1.5, 31, 'ab', 1000)); }
    public function testRegexAndMb(): void { $this->assertMatchesRegularExpression('/\p{Lu}\w+/u', 'Élan'); $this->assertSame(4, mb_strlen('Élan')); }
    public function testCount(): void { $this->assertCount(3, new \ArrayIterator([1,2])); }
    public function testContainsAndInstance(): void { $this->assertContains(3, [1,2]); }
    public function testFileAsserts(): void { $f = tempnam(sys_get_temp_dir(), 'x'); file_put_contents($f, "abc"); $this->assertStringEqualsFile($f, "abd"); }
    public function testXml(): void { $this->assertXmlStringEqualsXmlString('<a><b x="1"/></a>', '<a><b x="2"/></a>'); }
}
