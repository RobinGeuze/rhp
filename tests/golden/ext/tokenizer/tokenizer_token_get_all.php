<?php
function show($code, $flags = 0) {
    foreach (token_get_all($code, $flags) as $t) {
        if (is_array($t)) echo token_name($t[0]), '(', json_encode($t[1]), ',', $t[2], ') ';
        else echo json_encode($t), ' ';
    }
    echo "\n";
}
show("<?php \$a = &\$b; function &f(&\$x, &...\$y) { return \$a & \$b; } ?>\nhtml<?= 1 ?>");
show("<?php\n\$x = <<<EOT\n  a \$b {\$c[1]} \${d} \${e[2]} \$f[3] \$g[x] \$h->i \$j?->k \$l[-1] \$m[\$n]\n  EOT;\n");
show("<?php \$x = <<<'EOT'\nraw \$x\nEOT;\n\$y = <<<\"Q\"\nQ;\n");
show("<?php (int)\$a; ( integer )\$b; (real)\$c; (binary)\$d; (void)\$e; (bool)\$f; (boolean) \$g;");
show("<?php 0x7FFFFFFFFFFFFFFF; 0x8000000000000000; 9223372036854775807; 9223372036854775808; 1_000; 0o17; 0b101; 017; 1.5e3; .5; 1.; 089;");
show("<?php yield from \$a; yield  /* c */ from \$b; yield\nfrom \$c; enum Foo {} enum extends; \$enum; Enum::X; readonly(1); readonly class A {} private(set) \$x;");
show("<?php __halt_compiler(); rest of file\n<?php junk");
show("<?php # comment ?>\n<?php // c2\n/** doc */ /**x*/ /* c */ #[Attr] \\Foo\\Bar namespace\\X Foo\\Bar \\x");
show("<? echo 1;");
show("<?php \$a?->b; \$a->b->c; \$a::class; Foo::new(); \$a |> f(...); \$a <=> \$b; \$a ??= 1; \$a **= 2; \$a <> \$b; `ls \$x`; \"\$a\";'x'; b\"y\"; b'z';");
show('<?php $a->list; $a->class; Foo::class; Foo::list; class A { function list() {} const CLASS = 1; public $x; use T { list as protected; } } f(array: 1); $a->array[0]; Foo::match(); new class {}; enum E: string { case Default = "x"; }', TOKEN_PARSE);
show('<?php $a->list; $a->class; Foo::class; Foo::list; class A { function list() {} const CLASS = 1; public $x; use T { list as protected; } } f(array: 1); $a->array[0]; Foo::match(); new class {}; enum E: string { case Default = "x"; }');
show("<?php \"\x01\" \x01 \x7f é; \$é;");
show("<?php \"abc");
show("<?php /* unterminated");
show("<?php\r\n\$a;\r\n?>\r\nx");
show("<?php ?>");
show("<?php");
show("text only");
show("<?php \$x = \"a{\$b->c['d']}e\${f}g\";");
show("<?php \$x = \"\$a[0x1] \$a[01] \$a[0] \$a[123456789012345678901] \$a[ 1]\";");
echo token_name(402), token_name(1), token_name(256), token_name(259), token_name(T_PIPE), token_name(411), token_name(412), "\n";
