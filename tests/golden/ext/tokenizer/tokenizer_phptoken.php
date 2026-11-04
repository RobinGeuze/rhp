<?php
$code = "<?php\n// hi\nfunction f(\$a) { return \$a + 1; }\necho \"x{\$y}\";\n";
foreach (PhpToken::tokenize($code) as $t) {
    echo $t->id, ' ', var_export($t->getTokenName(), true), ' ', json_encode($t->text), ' L', $t->line, ' @', $t->pos,
        $t->isIgnorable() ? ' ignorable' : '', "\n";
}
$toks = PhpToken::tokenize('<?php $x = 1;');
$v = $toks[1];
var_dump($v instanceof Stringable, (string) $v, $v->is(T_VARIABLE), $v->is('$x'), $v->is([T_STRING, '$x']), $v->is([T_STRING]), $v->is('x'));
try {
    $v->is(1.5);
} catch (TypeError $e) {
    echo $e->getMessage(), "\n";
}
try {
    $v->is([1.5]);
} catch (TypeError $e) {
    echo $e->getMessage(), "\n";
}
$t = new PhpToken(T_STRING, 'foo');
var_dump($t->id, $t->text, $t->line, $t->pos, $t->getTokenName());
var_dump((new PhpToken(9999, 'x'))->getTokenName(), (new PhpToken(ord(';'), ';'))->getTokenName());
class MyToken extends PhpToken {
    public function upper(): string { return strtoupper($this->text); }
}
$m = MyToken::tokenize('<?php echo foo;');
echo get_class($m[2]), ' ', $m[2]->upper(), ' ', $m[3]->upper(), "\n";
var_dump(count(PhpToken::tokenize('')), PhpToken::tokenize('<?php Foo::list;', TOKEN_PARSE)[3]->getTokenName());
var_dump(token_name(T_STRING), token_name(T_DOUBLE_COLON), token_name(T_PAAMAYIM_NEKUDOTAYIM), token_name(-1), token_name(59));
var_dump(T_STRING, T_VARIABLE, T_OPEN_TAG, T_BAD_CHARACTER, T_PIPE, T_VOID_CAST, TOKEN_PARSE, T_DOUBLE_COLON === T_PAAMAYIM_NEKUDOTAYIM);
var_dump(extension_loaded('tokenizer'), function_exists('token_get_all'));
