#!/usr/bin/env php
<?php
echo __LINE__, " ", $argi, ":", $argn, "\n";
if ($argi == 3) throw new Exception("from file");
