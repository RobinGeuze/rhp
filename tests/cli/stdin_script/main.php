<?php
echo __FILE__, "\n", __LINE__, "\n", basename(__DIR__) === basename(getcwd()) ? "cwd" : "other", "\n";
var_dump($argv, $argc, $_SERVER['SCRIPT_FILENAME'], $_SERVER['PHP_SELF'], get_included_files());
var_dump(fgets(STDIN));
trigger_error("note");
throw new Exception("boom");
