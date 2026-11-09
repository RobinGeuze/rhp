<?php
// Method calls by name: a class with more methods than a small cache
// holds, called in turn; `$o->$name()`, `[$o, 'm']()`, call_user_func
// and method_exists, as dev tools' dispatch tables do.
class Big {
    public int $n = 0;
    function m00() { return 0; } function m01() { return 1; } function m02() { return 2; } function m03() { return 3; }
    function m04() { return 4; } function m05() { return 5; } function m06() { return 6; } function m07() { return 7; }
    function m08() { return 8; } function m09() { return 9; } function m10() { return 10; } function m11() { return 11; }
    function m12() { return 12; } function m13() { return 13; } function m14() { return 14; } function m15() { return 15; }
    function m16() { return 16; } function m17() { return 17; } function m18() { return 18; } function m19() { return 19; }
    function m20() { return 20; } function m21() { return 21; } function m22() { return 22; } function m23() { return 23; }
    function m24() { return 24; } function m25() { return 25; } function m26() { return 26; } function m27() { return 27; }
    function m28() { return 28; } function m29() { return 29; } function m30() { return 30; } function m31() { return 31; }
    function all() {
        return $this->m00() + $this->m01() + $this->m02() + $this->m03() + $this->m04() + $this->m05()
            + $this->m06() + $this->m07() + $this->m08() + $this->m09() + $this->m10() + $this->m11()
            + $this->m12() + $this->m13() + $this->m14() + $this->m15() + $this->m16() + $this->m17()
            + $this->m18() + $this->m19() + $this->m20() + $this->m21() + $this->m22() + $this->m23()
            + $this->m24() + $this->m25() + $this->m26() + $this->m27() + $this->m28() + $this->m29()
            + $this->m30() + $this->m31();
    }
}
$o = new Big();
$names = [];
for ($j = 0; $j < 32; $j++) { $names[] = sprintf('M%02d', $j); }
$t = 0;
for ($i = 0; $i < 150000; $i++) {
    $t += $o->all();
    $name = $names[$i & 31];
    $t += $o->$name() + [$o, $name]() + call_user_func([$o, $name]);
    $t += method_exists($o, $name) ? 1 : 0;
    $t &= 0xffffff;
}
echo $t, "\n";
