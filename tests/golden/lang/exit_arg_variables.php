<?php
// Variables first used inside exit()/die()/eval() are real variables: the
// temporary holding `new App` must not alias $_SERVER or $code.
class App
{
    public function run(array $argv): int
    {
        echo 'argv is ', gettype($argv), "\n";
        echo 'server still ', gettype($GLOBALS['_SERVER'] ?? $_SERVER), "\n";
        return 3;
    }
}
function viaEval()
{
    return eval($code = 'return (new App) instanceof App;') && $code !== '';
}
var_dump(viaEval());
register_shutdown_function(function () {
    echo "shutdown sees ", gettype($_SERVER), "\n";
});
exit((new App)->run($_SERVER['argv']));
