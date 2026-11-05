<?php
class Counter {
    public $count = 0;
    private $step;
    public function __construct($start, $step = 1) {
        $this->count = $start;
        $this->step = $step;
    }
    public function inc() {
        $this->count += $this->step;
        return $this;
    }
    public function get() {
        return $this->count;
    }
}
$c = new Counter(10, 5);
$c->inc()->inc();
echo $c->get(), "\n";
echo $c->count, "\n";
$d = new Counter(100);
$d->inc();
echo $d->get(), "\n";
echo $c->get(), "\n";
