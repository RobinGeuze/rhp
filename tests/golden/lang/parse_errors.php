<?php
function t($code) { try { eval($code); echo "ok\n"; } catch (ParseError $e) { echo get_class($e), ": ", $e->getMessage(), " @", $e->getLine(), "\n"; } }
t('$x = ;');
t("echo 1;\n\nfunction (");
t("if (1) {\n echo 2;\n");
t('foo());');
t("\$a = [1, 2);");
t('$a = 1 +;');
t('$x = 1');
t('echo 1; } ');
t('#[Attr( ');
t('return 7;');
var_dump(eval('return 7;'));
try { eval('$x = ;'); } catch (Error $e) { echo get_class($e), " ", get_parent_class($e), " ", $e instanceof CompileError ? "ce" : "", "\n"; }
try { include __DIR__ . '/parse_error_bad.inc'; } catch (ParseError $e) { echo "include: ", $e->getMessage(), " in ", basename($e->getFile()), ":", $e->getLine(), "\n"; }
try { eval('$y = ;'); } catch (ParseError $e) { echo "eval file: ", str_replace(__FILE__, 'F', $e->getFile()), "\n"; }
function inner() { return eval('return 1 +;'); }
try { inner(); } catch (ParseError $e) { echo "in function: ", $e->getMessage(), "\n"; }
echo "done\n";
