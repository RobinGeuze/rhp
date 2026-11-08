<?php class D { function __destruct() { throw new Exception("dtor"); } } $d = new D; echo "body\n";
