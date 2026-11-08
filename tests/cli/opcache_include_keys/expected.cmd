# opcache finds an included script by the name asked for (absolute with
# '..' or './', or relative, qualified by cwd, include_path and the
# includer's directory) and adds each name as a key; outside the
# revalidate window it resolves the name again, so a symlink swapped to
# another target is followed once the realpath cache forgets the old one.
ini opcache.enable_cli=1
ini opcache.file_update_protection=0
ini opcache.revalidate_freq=0
ini include_path=.
