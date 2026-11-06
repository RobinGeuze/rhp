<?php
// String offset writes and variable-variable names convert objects with
// __toString (after the offset's own checks).
class S { function __toString(): string { echo "toString\n"; return "xy"; } }
class T { function __toString(): string { throw new Exception("no"); } }
$s = "abc";
var_dump($s[1] = new S);
var_dump($s);
$s[5] = new S;
var_dump($s);
try { $s["1.5"] = new T; } catch (Exception $e) { echo $e->getMessage(), "\n"; }
var_dump($s);
try { $s[0] = function () {}; } catch (Error $e) { echo $e->getMessage(), "\n"; }
$xy = "var";
var_dump(${new S});
${new S} = "set";
var_dump($xy);
var_dump(isset(${new S}));
unset(${new S});
var_dump(isset($xy));
try { var_dump(${new T}); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
