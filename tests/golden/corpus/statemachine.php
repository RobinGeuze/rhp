<?php
enum State {
    case Idle;
    case Running;
    case Paused;
    case Stopped;
}
class Machine {
    private State $state = State::Idle;
    private array $log = [];
    public function transition(string $event): void {
        $next = match ([$this->state, $event]) {
            default => match (true) {
                $this->state === State::Idle && $event === 'start' => State::Running,
                $this->state === State::Running && $event === 'pause' => State::Paused,
                $this->state === State::Paused && $event === 'resume' => State::Running,
                $this->state === State::Running && $event === 'stop' => State::Stopped,
                default => $this->state,
            },
        };
        $this->log[] = $this->state->name . "->" . $next->name;
        $this->state = $next;
    }
    public function dump(): string { return implode("\n", $this->log); }
}
$m = new Machine();
foreach (['start', 'pause', 'resume', 'stop', 'start'] as $e) $m->transition($e);
echo $m->dump(), "\n";
