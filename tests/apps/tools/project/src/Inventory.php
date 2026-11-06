<?php

namespace App;

// Deliberate issues for the analysers, fixers and rector: old array syntax,
// strpos() !== false, a missing return type, an unused variable, a call to a
// method that doesn't exist, a wrong @return type and a possibly-null access.
class Inventory
{
    private $items;
    private $name;

    public function __construct($name, array $items = array())
    {
        $this->name = $name;
        $this->items = $items;
    }

    public function name()
    {
        return $this->name;
    }

    public function add(string $sku, int $quantity): void
    {
        $previous = $this->items[$sku] ?? 0;
        $unused = $previous * 2;
        $this->items[$sku] = $previous + $quantity;
    }

    /**
     * @return string
     */
    public function count(string $sku): int
    {
        return $this->items[$sku] ?? 0;
    }

    public function hasPrefix(string $prefix): bool
    {
        foreach (array_keys($this->items) as $sku) {
            if (strpos($sku, $prefix) !== false) {
                return true;
            }
        }
        return false;
    }

    public function total(): int
    {
        return $this->sumAll();
    }

    public function find(string $sku): ?Item
    {
        if (!isset($this->items[$sku])) {
            return null;
        }
        return new Item($sku, $this->items[$sku]);
    }

    public function describe(string $sku): string
    {
        return $this->find($sku)->label();
    }
}
