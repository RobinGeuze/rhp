<?php
echo urlencode("a b&c=d"), "\n";
echo rawurlencode("a b~c"), "\n";
echo htmlspecialchars("<a href='x'>\"&\"</a>"), "\n";
echo nl2br("line1\nline2"), "\n";
echo addslashes("O'Reilly \"quote\""), "\n";
echo urldecode("a%20b%2Bc"), "\n";
