<?php
// glob(): php's bundled (OpenBSD) glob — braces, sets, classes, escapes,
// GLOB_MARK/ONLYDIR/NOCHECK/NOSORT/NOESCAPE/ERR, dot files, symlinks.
function rm_tree($p) {
    if (is_link($p) || is_file($p)) { unlink($p); return; }
    if (!is_dir($p)) return;
    foreach (scandir($p) as $e) {
        if ($e !== '.' && $e !== '..') rm_tree("$p/$e");
    }
    rmdir($p);
}

function show($base, $r) {
    if ($r === false) return "false";
    return json_encode(array_map(fn($p) => str_starts_with($p, $base) ? "~" . substr($p, strlen($base)) : $p, $r));
}

function main() {
    $base = sys_get_temp_dir() . "/rhp_glob_" . bin2hex(random_bytes(6));
    mkdir($base);
    foreach (["a.txt", "b.txt", "c.php", ".hidden", "B.TXT", "x1", "x2", "x10", "[ab]", "q?", "a{b", "a,b"] as $f) {
        touch("$base/$f");
    }
    mkdir("$base/sub");
    mkdir("$base/sub/deep");
    touch("$base/sub/s.txt");
    touch("$base/sub/deep/d.txt");
    mkdir("$base/dir2");
    touch("$base/dir2/z.txt");
    symlink("$base/sub", "$base/lnk");
    symlink("$base/nowhere", "$base/dangling");

    $pats = ["*", "*.txt", "*.{txt,php}", "{a,b}.txt", "x?", "x[0-9]", "x[!1]*", "x[^1]*",
        "[[:upper:]]*", "*/*.txt", "*/*/*", ".*", "s*/", "nomatch*", "nomatch", "\\[ab]", "q\\?",
        "a{b", "a\\{b", "{", "{}", "*{", "[ab]", "[a-", "x[1-]", "x[]1]*", "sub/../*.php", "*/", "**",
        "[[:alpha:][:digit:]]*", "[[:bogus:]]*", "{x1,a.txt,nomatch}", "{sub,dir2}/*.txt",
        "{{a,b},c}.*", "x{1,2}{0,}", "a{\\,,.}b", "{a,b}{.txt,}", "d*", "l*/*", "sub//s.txt", "*/../c.php"];
    $flags = [0, GLOB_BRACE, GLOB_MARK, GLOB_NOSORT | GLOB_BRACE, GLOB_NOCHECK, GLOB_ONLYDIR,
        GLOB_NOESCAPE, GLOB_ERR | GLOB_MARK | GLOB_BRACE, GLOB_ONLYDIR | GLOB_MARK];
    foreach ($pats as $p) {
        echo "== $p\n";
        foreach ($flags as $fl) {
            $r = glob("$base/$p", $fl);
            if ($r !== false && ($fl & GLOB_NOSORT)) sort($r);
            echo "  $fl: ", show($base, $r), "\n";
        }
    }
    // relative-free edge cases
    var_dump(@glob("$base/*", 1 << 20));             // unsupported flag → false
    var_dump(glob(""));
    var_dump(glob("", GLOB_NOCHECK));
    try { glob("a\0b"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
    var_dump(GLOB_BRACE, GLOB_MARK, GLOB_NOSORT, GLOB_NOCHECK, GLOB_NOESCAPE, GLOB_ERR, GLOB_ONLYDIR, GLOB_AVAILABLE_FLAGS);

    rm_tree($base);
}

try { main(); } catch (Throwable $e) { echo "Uncaught: ", $e->getMessage(), "\n"; }
