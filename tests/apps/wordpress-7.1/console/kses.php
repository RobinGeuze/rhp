<?php
echo wp_kses_post('<a href="javascript:x">y</a><script>z</script><p onclick=1 style="color:red;position:fixed">ok</p><img src=x onerror=y>'), "\n";
echo wp_kses('<b>b</b><i title="t">i</i>', array('b'=>array())), "\n";
echo esc_html('<&"\'>'), esc_attr("a\"b"), esc_url('http://ex.com/a b?x=<1>'), esc_js("a'b\n"), "\n";
echo sanitize_text_field("  a\tb <b>x</b> %AB "), '|', sanitize_file_name('Héllo wörld?.php.jpg'), '|', sanitize_key('Ab-C_d!'), '|', sanitize_email('a@@b.com'), "\n";
echo wp_strip_all_tags('<p>a</p><style>x</style>b'), "\n";
echo balanceTags('<b><i>x</b>', true), "\n";
echo make_clickable('see http://example.com/x?y=1 and www.foo.org and a@b.co'), "\n";
echo convert_smilies(':) :-('), "\n";
echo wp_html_excerpt('<p>Some <b>bold</b> text here</p>', 10, '…'), "\n";
