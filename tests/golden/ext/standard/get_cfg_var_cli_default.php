<?php
// sapi_cli_ini_defaults: the CLI puts display_errors=1 in the configuration
// hash, so get_cfg_var() sees it without any ini file.
var_dump(get_cfg_var("display_errors"), get_cfg_var("html_errors"), get_cfg_var("no_such_entry"));
