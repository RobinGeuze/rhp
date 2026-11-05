<?php
set_error_handler(function ($no, $msg) { echo "[$no] $msg\n"; return true; });
// formatted_print.c details beyond the phpt corpus.
printf("[%h] [%H] [%.3h] [%.*h]\n", 1e20, 1e-7, 123456.0, -1, 0.1);
printf("[%'*10s] [%-'x10d] [%010.3f] [%-010.3f] [%+05d] [%+.1e]\n", "abc", 42, -3.14159, 2.5, 7, 12345.678);
printf("[%*d] [%-*d] [%.*f] [%2\$*1\$d]\n", 5, 42, 5, 42, 2, 3.14159);
printf("[%.2x] [%5.1o] [%.0b] [%u] [%c]\n", 255, 8, 5, -1, 65);
printf("[%e] [%.0e] [%E] [%g] [%G] [%g]\n", 0.0, 12345.0, -1.5e-10, 1e-5, 1e15, -0.0);
printf("[%5s] [%-5s|] [%.2s] [%05s]\n", "ab", "ab", "abcdef", "ab");
printf("[%F] [%.60f]\n", 1.5, 0.1);
foreach (['%', '%5', '%0$s', '%*s', '%.*s', '%y', "%'"] as $f) {
    try { var_dump(sprintf($f, 'x')); } catch (Throwable $t) { echo get_class($t), ": ", $t->getMessage(), "\n"; }
}
try { vsprintf("%d %d", [1]); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { sprintf("%d %d", 1); } catch (ArgumentCountError $e) { echo $e->getMessage(), "\n"; }
try { sprintf("%.*f", -2, 1.0); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { sprintf("%.*d", -1, 1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
try { sprintf("%*d", "5", 1); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
ini_set('precision', 20);
echo 0.1, " ", 1/3, " ", -1e-300, "\n";
ini_set('precision', 14);
