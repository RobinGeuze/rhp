# An include_path without '.', and y.php in neither it nor app/: only the
# plain open relative to the working directory finds it.
cwd cwd
script ../app/m.php
ini display_errors=1
ini error_reporting=-1
