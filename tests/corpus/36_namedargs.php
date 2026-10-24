<?php
function makeUser(string $name, int $age = 0, string $role = 'user', bool $active = true): string {
    return "$name/$age/$role/" . ($active ? 'Y' : 'N');
}
echo makeUser("Alice"), "\n";
echo makeUser("Bob", role: 'admin'), "\n";
echo makeUser("Carol", active: false, age: 30), "\n";
echo makeUser(name: "Dave", role: 'mod', age: 25, active: true), "\n";

class Point {
    public function __construct(
        public int $x = 0,
        public int $y = 0,
        public int $z = 0,
    ) {}
    public function __toString(): string { return "($this->x,$this->y,$this->z)"; }
}
echo new Point(y: 5), "\n";
echo new Point(1, z: 9), "\n";

$data = str_replace(search: "o", replace: "0", subject: "foo bar");
echo $data, "\n";
echo array_sum([1, 2, 3]), "\n";
