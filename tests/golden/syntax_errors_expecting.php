<?php
// php's own parser decides and words syntax errors (Bison's "expecting"
// lists, at most four), and its scanner raises its own errors first.
function t($code) {
    try {
        eval($code);
        echo "ok\n";
    } catch (ParseError $e) {
        echo $e->getMessage(), " @", $e->getLine(), "\n";
    }
}
t('echo 1');
t('$a = [1, 2;');
t('class A { public function }');
t('function f() { return 1 }');
t('echo $a->;');
t('use A\{B, ;');
t('echo "$a[";');
t('$f = function() use { };');
t("\$x = 1\n\$y = 2;");
t('foo bar;');
t('echo \a\;');
t('$a = 1 & & 2;');
t('echo "abc');
t('$x = `ls ;');
t("echo <<<EOT\nabc\nEOT 1;");
t('$a = \'abcdefghijklmnopqrstuvwxyz0123456789abc\' 1;');
t('$a = "x" "y";');
t('$a = b\'x\' 1;');
t("#[Attr\nfunction f() {}");
t('if (1): echo 1; endwhile;');
t("\x01");
t('$x = 09;');
t('$x = (real) 1;');
t('/* abc');
t('echo "\u{zz}";');
t('echo "\u{110000}";');
t('(]');
t("echo 1;\n}");
t("echo <<<END\n      a\n     b\n     END;");
t("echo <<<END\n\t   a\n\t\tEND;");
t('__halt_compiler(); }}}}');
t('return 7;');
try {
    token_get_all("<?php\n\n1 1;", TOKEN_PARSE);
} catch (ParseError $e) {
    var_dump($e->getMessage(), $e->getLine(), $e->getFile());
}
