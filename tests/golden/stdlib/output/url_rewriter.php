<?php
ini_set("url_rewriter.tags", "a=href,area=href,frame=src,form=,fieldset=");
ini_set("url_rewriter.hosts", "php.net");
output_add_rewrite_var("a","b");
echo "<a href=\"x.php\">y</a><form action=\"q.php\"></form><a href=\"http://example.com/x\">z</a>\n";
echo "<a href=x>"; echo "<A HREF='x?q=1#f'>"; echo "<form action=\"http://h/\" method=post>"; echo "<form action>"; echo "<a href = \"y\" >"; echo "<form>";
ob_flush();
echo "<a href=\"\"><form/>";
output_reset_rewrite_vars();
echo "<a href=\"zz\">";
output_add_rewrite_var("c d","<e>");
echo "<form>x<a href='//php.net/'>";
