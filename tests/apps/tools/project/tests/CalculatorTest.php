<?php

declare(strict_types=1);

namespace Tests;

use App\Calculator;
use PHPUnit\Framework\Attributes\DataProvider;
use PHPUnit\Framework\TestCase;

final class CalculatorTest extends TestCase
{
    public function testAdd(): void
    {
        $this->assertSame(5, (new Calculator())->add(2, 3));
    }

    /**
     * @return array<string, array{int, int, float}>
     */
    public static function divisions(): array
    {
        return [
            'whole' => [6, 3, 2.0],
            'fraction' => [1, 4, 0.25],
            'negative' => [-9, 3, -3.0],
        ];
    }

    #[DataProvider('divisions')]
    public function testDivide(int $a, int $b, float $expected): void
    {
        $this->assertSame($expected, (new Calculator())->divide($a, $b));
    }

    public function testDivideByZero(): void
    {
        $this->expectException(\DivisionByZeroError::class);
        (new Calculator())->divide(1, 0);
    }

    public function testAverage(): void
    {
        $this->assertSame(2.5, (new Calculator())->average([1, 2, 3, 4]));
        $this->assertSame(0.0, (new Calculator())->average([]));
    }
}
