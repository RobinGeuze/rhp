<?php
function g() { throw new Exception("from the appended file"); }
echo "appended\n";
g();
