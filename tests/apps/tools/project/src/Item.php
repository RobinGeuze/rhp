<?php
namespace App;

class Item {
    public function __construct(private string $sku, private int $quantity) {}

    public function label() : string {
        if($this->quantity==0) return "out of stock: ".$this->sku;
        return sprintf("%s x %d", $this->sku, $this->quantity);
    }

    public function isEmpty(){
        return $this->quantity === 0 ? true : false;
    }
}
