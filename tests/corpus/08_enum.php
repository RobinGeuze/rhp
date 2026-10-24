<?php
enum Suit: string {
    case Hearts = 'H';
    case Diamonds = 'D';
    case Clubs = 'C';
    case Spades = 'S';
    public function color(): string {
        return match ($this) {
            Suit::Hearts, Suit::Diamonds => 'red',
            Suit::Clubs, Suit::Spades => 'black',
        };
    }
    public static function fromChar(string $c): self {
        return self::from($c);
    }
}
foreach (Suit::cases() as $s) {
    echo "{$s->name} ({$s->value}): {$s->color()}\n";
}
echo Suit::fromChar('S')->name, "\n";
echo Suit::tryFrom('X')?->name ?? 'none', "\n";

enum Status {
    case Active;
    case Inactive;
}
$x = Status::Active;
echo ($x === Status::Active) ? "active\n" : "other\n";
echo count(Status::cases()), "\n";
