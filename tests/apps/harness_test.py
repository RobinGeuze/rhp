#!/usr/bin/env python3
"""Unit tests of tools.py's and composer_diff.py's comparison helpers.

    python3 tests/apps/harness_test.py
"""

import sys
import unittest
from pathlib import Path

sys.dont_write_bytecode = True  # no __pycache__ in the tree
sys.path.insert(0, str(Path(__file__).resolve().parent))
import composer_diff  # noqa: E402
import tools  # noqa: E402

CS_A = """Loaded config default.
   1) src/B.php
      ---------- begin diff ----------
-x
+y
      ----------- end diff -----------

   2) src/A.php
      ---------- begin diff ----------
-a
+b
      ----------- end diff -----------


Found 2 of 5 files that can be fixed in 0.1 seconds, 20.00 MB memory used
"""

CS_B = """Loaded config default.
   1) src/A.php
      ---------- begin diff ----------
-a
+b
      ----------- end diff -----------

   2) src/B.php
      ---------- begin diff ----------
-x
+y
      ----------- end diff -----------


Found 2 of 5 files that can be fixed in 0.3 seconds, 21.00 MB memory used
"""

RECTOR_A = """2 files with changes
====================

1) src/B.php:3

    ---------- begin diff ----------
@@ Line 3 @@
-x
+y
    ----------- end diff -----------

Applied rules:
 * FooRector


2) src/A.php:7

    ---------- begin diff ----------
@@ Line 7 @@
-a
+b
    ----------- end diff -----------

Applied rules:
 * BarRector


 [OK] 2 files would have been changed (dry-run) by Rector
"""


class Unorder(unittest.TestCase):
    def test_text_blocks_sorted(self):
        self.assertEqual(tools.unorder(CS_A).split("Found")[0], tools.unorder(CS_B).split("Found")[0])

    def test_text_keeps_content(self):
        a = tools.unorder(CS_A)
        self.assertIn("Loaded config default.\n", a)
        self.assertEqual(a.count("#) src/"), 2)
        self.assertTrue(a.rstrip().endswith("memory used"))
        self.assertNotEqual(tools.unorder(CS_A), tools.unorder(CS_A.replace("+b", "+c")))

    def test_number_padding(self):
        a = "   9) src/A.php\n  10) src/B.php\nFound 2 of 2 files\n"
        b = "   9) src/B.php\n  10) src/A.php\nFound 2 of 2 files\n"
        self.assertEqual(tools.unorder(a), tools.unorder(b))

    def test_rector_blocks(self):
        swapped = RECTOR_A.replace("src/B.php:3", "TMP").replace("src/A.php:7", "src/B.php:3").replace("TMP", "src/A.php:7")
        swapped = swapped.replace("FooRector", "TMP").replace("BarRector", "FooRector").replace("TMP", "BarRector")
        swapped = swapped.replace("Line 3", "TMP").replace("Line 7", "Line 3").replace("TMP", "Line 7")
        swapped = swapped.replace("-x\n+y", "TMP").replace("-a\n+b", "-x\n+y").replace("TMP", "-a\n+b")
        self.assertEqual(tools.unorder(RECTOR_A), tools.unorder(swapped))
        self.assertIn(" [OK] 2 files", tools.unorder(RECTOR_A))

    def test_json_sorted_by_file(self):
        a = '{"files":[{"name":"b.php","diff":"1"},{"name":"a.php","diff":"2"}],"changed_files":["b","a"]}'
        b = '{"files":[{"name":"a.php","diff":"2"},{"name":"b.php","diff":"1"}],"changed_files":["a","b"]}'
        self.assertEqual(tools.unorder(a), tools.unorder(b))

    def test_json_keeps_other_lists(self):
        a = '{"rules":["b","a"]}'
        self.assertNotEqual(tools.unorder(a), tools.unorder('{"rules":["a","b"]}'))

    def test_plain_text_unchanged(self):
        self.assertEqual(tools.unorder("no blocks here\n"), "no blocks here\n")


class Normalise(unittest.TestCase):
    def test_worker_label(self):
        t = tools.normalise("peak 10 MB (the spawned worker)", [])
        self.assertEqual(t, tools.normalise("peak 12 MB (the forked worker)", []))
        self.assertEqual(
            tools.normalise("(largest of 2 spawned workers)", []), tools.normalise("(largest of 2 forked workers)", [])
        )

    def test_binary_paths_absolute_only(self):
        self.assertTrue(all(Path(b).is_absolute() for b in tools.binary_paths(sys.executable, sys.executable)))


class Platform(unittest.TestCase):
    def test_owner(self):
        self.assertEqual(composer_diff.platform_owner("ext-mbstring"), "mbstring")
        self.assertEqual(composer_diff.platform_owner("lib-curl-openssl"), "curl")
        self.assertEqual(composer_diff.platform_owner("lib-icu-uca"), "intl")
        self.assertEqual(composer_diff.platform_owner("lib-pdo_sqlite-sqlite"), "pdo_sqlite")
        self.assertIsNone(composer_diff.platform_owner("php-64bit"))

    def test_only_common(self):
        text = "composer   2.11.1  Composer package\next-gd     8.5.12  The gd PHP extension\nlib-icu    79.1    The ICU unicode\next-json   8.5.12  The json PHP extension\n"
        self.assertEqual(
            composer_diff.only_common(text, {"json"}),
            "composer 2.11.1 Composer package\next-json 8.5.12 The json PHP extension\n",
        )

    def test_zlib_not_masked(self):
        # rhp reports the zlib it links, the one php links (libraries.md).
        a = composer_diff.mask("lib-zlib 1.3.1 The zlib library\n", [])
        self.assertNotEqual(a, composer_diff.mask("lib-zlib 1.2.12 The zlib library\n", []))

    def test_openssl_masked(self):
        a = composer_diff.mask("OpenSSL version: OpenSSL 4.0.3 29 Sep 2026\nlib-openssl 4.0.3 OpenSSL 4.0.3\n", [])
        b = composer_diff.mask("OpenSSL version: OpenSSL 4.1.0 23 Oct 2026\nlib-openssl 4.1.0 OpenSSL 4.1.0\n", [])
        self.assertEqual(a, b)


if __name__ == "__main__":
    unittest.main()
