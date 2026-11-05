<?php
// scanf.c corners php-src's tests leave open
function show($v) { echo json_encode($v), "\n"; }
show(sscanf("-5", "%u"));
show(sscanf("18446744073709551615", "%u"));
show(sscanf("99999999999999999999", "%d"));
show(sscanf("-99999999999999999999", "%d"));
show(sscanf("0x1f 017 19", "%i %i %i"));
show(sscanf("0x31", "%x"));
show(sscanf("-0x31", "%x"));
show(sscanf("0xg", "%x%s"));
show(sscanf("12", "%ld"));
show(sscanf("1.5", "%lf"));
show(sscanf("abc", "%hs"));
show(sscanf("1e5 2e 3e+ .5 -.e1", "%f %f%s %f%s %f %f"));
show(sscanf("12345678", "%3d%2d%d"));
show(sscanf("hello world", "%s%n %c%n"));
show(sscanf("a b c", "%3\$s %1\$s %2\$s"));
show(sscanf("x y", "%2\$s"));
show(sscanf("abc]def", "%[]a-c]%s"));
show(sscanf("zyx-abc", "%[z-x]%[-]%[^c]"));
show(sscanf("abc", "%*s%n"));
show(sscanf("", "%d"));
show(sscanf("  ", "%d"));
show(sscanf("x", "x%d"));
show(sscanf("ab\0cd", "%s"));
show(sscanf(str_repeat("1", 70), "%d"));
var_dump(sscanf("", "%d", $a), $a);
var_dump(sscanf("7 8", "%d %d", $a, $b), $a, $b);
var_dump(sscanf("7", "%d %d", $a, $b), $a, $b);
var_dump(sscanf("1 2", "%2\$d %1\$d", $a, $b), $a, $b);
foreach (["%", "%h", "%q", "%[", "%[^", "%[]", "%1\$s %s", "%s %1\$s", "%0\$s", "%256\$s", "%1\$s%1\$s"] as $f) {
    try {
        var_dump(sscanf("a b", $f));
    } catch (ValueError $e) {
        echo json_encode($f), ": ", json_encode($e->getMessage()), "\n";
    }
}
try { sscanf("a", "%s", $a, $b); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { sscanf("a", "%s %s", $a); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { sscanf("a", "%2\$s", $a); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
$f = tmpfile();
fwrite($f, "10 apples\n20 pears\nlast");
rewind($f);
while (($r = fscanf($f, "%d %s")) !== false) { show($r); }
rewind($f);
var_dump(fscanf($f, "%d %s", $n, $what), $n, $what);
fclose($f);
try { fscanf($f, "%d"); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
