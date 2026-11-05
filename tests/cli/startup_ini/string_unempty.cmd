# OnUpdateStringUnempty directives refuse an empty value: the default
# stays, at startup and in ini_set.
noscript
ini variables_order=
ini sys_temp_dir=
opt -r
opt var_dump(ini_get('variables_order'), isset($_SERVER['argv']), ini_get('sys_temp_dir'), ini_set('include_path', ''), get_include_path() !== '');
