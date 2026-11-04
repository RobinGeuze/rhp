<?php
class Config {
    const SECONDS_PER_HOUR = 60 * 60;
    const GREETING = "Hello" . ", " . "World";
    const FLAGS = (1 << 4) | 2;
    const NEG = -5 * 3;
    const IS_BIG = 100 > 50;
    const COMPUTED = [1 + 1, 2 * 3, "a" . "b"];
    const NAME_LEN = 4 + 2;
}
echo Config::SECONDS_PER_HOUR, "\n";
echo Config::GREETING, "\n";
echo Config::FLAGS, " ", Config::NEG, " ", Config::NAME_LEN, "\n";
var_dump(Config::IS_BIG);
echo implode(",", Config::COMPUTED), "\n";

function shift(int $x, int $by = 2 + 1): int { return $x << $by; }
echo shift(1), " ", shift(1, 4), "\n";

class Defaults {
    public int $limit = 10 * 5;
    public string $sep = "-" . "-";
    public bool $on = 1 === 1;
}
$d = new Defaults();
echo $d->limit, " ", $d->sep, " ", var_export($d->on, true), "\n";


const APP_VERSION = 2;
const APP_NAME = "phrust" . "-engine";
echo APP_NAME, " v", APP_VERSION, "\n";
