<?php
enum State { case Idle; case Running; case Done;
    public function label(): string { return ucfirst(strtolower($this->name)); }
}
class Machine {
    public State $state = State::Idle;
    const DEFAULT = State::Running;
    public static State $shared = State::Done;
}
$m = new Machine();
echo $m->state->name, " ", $m->state->label(), "\n";
echo Machine::DEFAULT->name, " ", Machine::$shared->name, "\n";

enum Size: int { case S = 1; case L = 3; }
class Box {
    const BIGGEST = Size::L;
    public int $cap = Size::L->value * 10;
    public array $all = [State::Idle, State::Done];
}
$b = new Box();
echo Box::BIGGEST->value, " ", $b->cap, "\n";
echo implode(",", array_map(fn($s) => $s->name, $b->all)), "\n";

class Cfg { const A = 10; const B = self::A * 2; const C = self::B + self::A; }
echo Cfg::A, " ", Cfg::B, " ", Cfg::C, "\n";
