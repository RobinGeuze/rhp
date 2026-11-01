<?php
// file(): FILE_IGNORE_NEW_LINES / FILE_SKIP_EMPTY_LINES / FILE_USE_INCLUDE_PATH,
// \r\n handling, trailing data without a newline, flag validation.
function main() {
    $base = sys_get_temp_dir() . "/rhp_file_" . bin2hex(random_bytes(6));
    mkdir($base);
    $cases = [
        "",
        "one",
        "one\n",
        "one\ntwo",
        "one\ntwo\n",
        "\n",
        "\n\n\n",
        "a\n\nb\n\n",
        "a\r\nb\r\n\r\nc",
        "a\r\n\r\n",
        "\r\n",
        "x\ry\rz",
        "trail\r",
        "\nlead",
        " \n\t\n",
        "a\0b\nc",
    ];
    $flagsets = [0, FILE_IGNORE_NEW_LINES, FILE_SKIP_EMPTY_LINES,
        FILE_IGNORE_NEW_LINES | FILE_SKIP_EMPTY_LINES, FILE_NO_DEFAULT_CONTEXT | FILE_IGNORE_NEW_LINES];
    foreach ($cases as $i => $c) {
        $f = "$base/c$i.txt";
        file_put_contents($f, $c);
        echo json_encode($c), "\n";
        foreach ($flagsets as $fl) {
            echo "  $fl: ", json_encode(file($f, $fl)), "\n";
        }
    }
    // include-path flag with an absolute path behaves like a plain read
    var_dump(file("$base/c3.txt", FILE_USE_INCLUDE_PATH | FILE_IGNORE_NEW_LINES));
    // missing file
    var_dump(@file("$base/missing.txt"));
    // invalid flags
    foreach ([32, -1, 1 << 10] as $bad) {
        try { file("$base/c1.txt", $bad); echo "no error\n"; }
        catch (ValueError $e) { echo $e->getMessage(), "\n"; }
    }
    try { file("a\0b"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
    var_dump(FILE_USE_INCLUDE_PATH, FILE_IGNORE_NEW_LINES, FILE_SKIP_EMPTY_LINES, FILE_NO_DEFAULT_CONTEXT);

    foreach (glob("$base/*") as $f) unlink($f);
    rmdir($base);
}

try { main(); } catch (Throwable $e) { echo "Uncaught: ", $e->getMessage(), "\n"; }
