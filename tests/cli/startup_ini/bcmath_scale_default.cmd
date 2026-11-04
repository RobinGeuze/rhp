# A startup bcmath.scale is bcmath's default scale, and ini_restore goes
# back to it.
noscript
ini bcmath.scale=4
opt -r
opt var_dump(bcscale(), ini_get('bcmath.scale')); echo bcadd('1', '2'), "\n"; ini_set('bcmath.scale', '2'); var_dump(bcscale()); ini_restore('bcmath.scale'); var_dump(bcscale(), ini_get_all('bcmath')['bcmath.scale']['global_value']);
