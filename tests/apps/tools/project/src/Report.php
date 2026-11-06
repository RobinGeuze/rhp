<?php

declare(strict_types=1);

namespace App;

final class Report
{
    public function __construct(private readonly Inventory $inventory)
    {
    }

    /**
     * @param list<string> $skus
     * @return array<string, int>
     */
    public function counts(array $skus): array
    {
        $out = [];
        foreach ($skus as $sku) {
            $out[$sku] = $this->inventory->count($sku);
        }
        return $out;
    }

    /**
     * @param list<string> $skus
     */
    public function render(array $skus): string
    {
        $lines = [];
        foreach ($this->counts($skus) as $sku => $count) {
            $lines[] = str_pad($sku, 10) . ' ' . $count;
        }
        return implode("\n", $lines) . "\n";
    }
}
