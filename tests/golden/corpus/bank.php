<?php
class InsufficientFunds extends Exception {}
class Account {
    private float $balance = 0.0;
    private array $history = [];
    public function __construct(public readonly string $owner) {}
    public function deposit(float $amt): static {
        if ($amt <= 0) throw new InvalidArgumentException("bad amount");
        $this->balance += $amt;
        $this->history[] = "+$amt";
        return $this;
    }
    public function withdraw(float $amt): static {
        if ($amt > $this->balance) {
            throw new InsufficientFunds("need $amt have {$this->balance}");
        }
        $this->balance -= $amt;
        $this->history[] = "-$amt";
        return $this;
    }
    public function balance(): float { return $this->balance; }
    public function statement(): string {
        return $this->owner . ": " . implode(" ", $this->history) . " = " . $this->balance;
    }
}
$acc = new Account("Alice");
$acc->deposit(100)->deposit(50)->withdraw(30);
echo $acc->statement(), "\n";
try {
    $acc->withdraw(1000);
} catch (InsufficientFunds $e) {
    echo "declined: " . $e->getMessage() . "\n";
}
echo $acc->owner, " ", number_format($acc->balance(), 2), "\n";
