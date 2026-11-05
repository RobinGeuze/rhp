<?php
$re = '~(/\*\*)(*MARK:1)|(\*/)(*MARK:2)|([a-z]+)(*MARK:3)|(\s+)(*MARK:4)~Asi';
preg_match_all($re, "/** foo */", $m, PREG_SET_ORDER);
var_dump($m);
preg_match('~a(*MARK:x)|b(*MARK:y)~', 'b', $mm); var_dump($mm);
preg_match_all('~a(*MARK:x)|b(*MARK:yy)|c~', 'abcab', $pm); var_dump($pm);
preg_match_all('~a(*MARK:x)|b~', 'ab', $pm2, PREG_SET_ORDER | PREG_OFFSET_CAPTURE); var_dump($pm2);
echo preg_replace_callback('~(a)(*MARK:A)|b(*MARK:B)~', function ($m) { return json_encode($m); }, 'ab'), "\n";
var_dump(preg_match('~(?<n>a)(*MARK:z)~', 'a', $nm), $nm);
