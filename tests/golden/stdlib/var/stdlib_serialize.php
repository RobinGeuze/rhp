<?php
echo serialize(null), "\n";
echo serialize(42), "\n";
echo serialize(3.14), "\n";
echo serialize("hello"), "\n";
echo serialize([1, "two", 3.5, true, null]), "\n";
echo serialize(["a"=>1, "b"=>[2, 3]]), "\n";
$r = unserialize('a:3:{i:0;i:1;i:1;s:3:"two";i:2;b:1;}');
var_dump($r);
$round = unserialize(serialize(["x"=>[1,2], "y"=>"z"]));
echo implode(",", $round["x"]), " ", $round["y"], "\n";
