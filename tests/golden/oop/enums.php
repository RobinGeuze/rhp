<?php
enum Suit {
    case Hearts;
    case Diamonds;
    case Clubs;
    case Spades;
    public function color(): string {
        return match($this) {
            Suit::Hearts, Suit::Diamonds => "Red",
            Suit::Clubs, Suit::Spades => "Black",
        };
    }
}
echo Suit::Hearts->name, "\n";
echo Suit::Hearts->color(), " ", Suit::Spades->color(), "\n";
var_dump(Suit::Hearts === Suit::Hearts);
var_dump(Suit::Hearts === Suit::Spades);
var_dump(Suit::Hearts instanceof Suit);
echo count(Suit::cases()), "\n";
foreach (Suit::cases() as $s) echo $s->name, " ";
echo "\n";

interface HasLabel { public function label(): string; }
enum Status: string implements HasLabel {
    case Active = 'active';
    case Inactive = 'inactive';
    public function label(): string { return ucfirst($this->value); }
    public static function default(): self { return self::Active; }
}
echo Status::Active->name, "=", Status::Active->value, "\n";
echo Status::Active->label(), "\n";
echo Status::default()->name, "\n";
echo Status::from('inactive')->name, "\n";
var_dump(Status::tryFrom('active'));
var_dump(Status::tryFrom('nope'));
var_dump(Status::Active instanceof UnitEnum);
var_dump(Status::Active instanceof BackedEnum);
var_dump(Status::Active instanceof HasLabel);

enum Size: int {
    case Small = 1;
    case Large = 3;
}
echo Size::Large->value, "\n";
echo Size::from(1)->name, "\n";
var_dump(Size::tryFrom(99));
try { Size::from(99); } catch (\ValueError $e) { echo "err: ", $e->getMessage(), "\n"; }
try { new Suit(); } catch (\Error $e) { echo "err: ", $e->getMessage(), "\n"; }
