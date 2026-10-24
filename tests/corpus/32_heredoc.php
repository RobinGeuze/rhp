<?php
$name = "World";
$items = ['a', 'b'];
$text = <<<EOT
Hello $name!
Array access: {$items[0]} and {$items[1]}
Upper: {$name}
Literal dollar: \$x
EOT;
echo $text, "\n";
$nowdoc = <<<'EOT'
No $interpolation here
Raw \n stays
EOT;
echo $nowdoc, "\n";
$x = 5;
echo <<<SQL
SELECT * FROM t WHERE id = $x AND name = 'test'
SQL;
echo "\n";
class Box { public string $prop = 'value'; }
$obj = new Box();
echo <<<T
Property: {$obj->prop}
T;
echo "\n";
