<?php

declare(strict_types=1);

namespace Tests;

use App\Inventory;
use App\Report;
use PHPUnit\Framework\Attributes\Group;
use PHPUnit\Framework\TestCase;

final class InventoryTest extends TestCase
{
    private Inventory $inventory;

    protected function setUp(): void
    {
        $this->inventory = new Inventory('shop', ['apple' => 3]);
    }

    public function testAdd(): void
    {
        $this->inventory->add('apple', 2);
        $this->inventory->add('pear', 1);
        $this->assertSame(5, $this->inventory->count('apple'));
        $this->assertSame(1, $this->inventory->count('pear'));
    }

    public function testHasPrefix(): void
    {
        $this->assertTrue($this->inventory->hasPrefix('app'));
        $this->assertFalse($this->inventory->hasPrefix('x'));
    }

    public function testDescribe(): void
    {
        $this->assertSame('apple x 3', $this->inventory->describe('apple'));
    }

    // Deliberately failing: the report pads to 10 columns.
    #[Group('failing')]
    public function testReportRender(): void
    {
        $report = new Report($this->inventory);
        $this->assertSame("apple 3\n", $report->render(['apple']));
    }

    public function testTotal(): void
    {
        $this->markTestSkipped('Inventory::total() is not implemented');
    }

    public function testFindMissingIsNull(): void
    {
        $this->assertNull($this->inventory->find('nope'));
    }
}
