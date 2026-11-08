"""Unit tests of suites.py: the TeamCity reader and the comparison.

    python3 -m unittest discover -s tests/suites -p 'test_*.py'
"""

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import suites  # noqa: E402

LOC = "php_qn:///w/tests/CalcTest.php::\\Tests\\CalcTest"

# What PHPUnit 13 writes (tests/apps/tools' phpunit fixture), trimmed.
LOG = f"""PHPUnit 13.4.6 by Sebastian Bergmann and contributors.

##teamcity[testCount count='7' flowId='42']
##teamcity[testSuiteStarted name='unit' flowId='42']
##teamcity[testSuiteStarted name='Tests\\CalcTest' locationHint='{LOC}' flowId='42']
##teamcity[testStarted name='testAdd' locationHint='{LOC}::testAdd' flowId='42']
##teamcity[testFinished name='testAdd' duration='1' flowId='42']
##teamcity[testSuiteStarted name='testDivide' locationHint='{LOC}::testDivide' flowId='42']
##teamcity[testStarted name='testDivide with data set "it|'s"' locationHint='{LOC}::testDivide with data set "it|'s"' flowId='42']
##teamcity[testFinished name='testDivide with data set "it|'s"' duration='1' flowId='42']
##teamcity[testStarted name='testDivide with data set #1' locationHint='{LOC}::testDivide with data set #1' flowId='42']
##teamcity[testFailed name='testDivide with data set #1' message='Failed asserting that 2 is identical to 3.|n' details=' /w/tests/CalcTest.php:20|n' flowId='42']
##teamcity[testFinished name='testDivide with data set #1' duration='1' flowId='42']
##teamcity[testSuiteFinished name='testDivide' flowId='42']
##teamcity[testStarted name='testSkipped' locationHint='{LOC}::testSkipped' flowId='42']
##teamcity[testIgnored name='testSkipped' message='needs redis' flowId='42']
##teamcity[testFinished name='testSkipped' duration='0' flowId='42']
##teamcity[testIgnored name='testNeverStarted' message='requires PHP 9' flowId='42']
##teamcity[testStarted name='testError' locationHint='{LOC}::testError' flowId='42']
##teamcity[testFailed name='testError' message='Error: Call to undefined function foo()' details='' flowId='42']
##teamcity[testFinished name='testError' duration='0' flowId='42']
##teamcity[testStarted name='testDies' locationHint='{LOC}::testDies' flowId='42']
"""


class TeamCity(unittest.TestCase):
    def test_outcomes(self):
        t = suites.parse_teamcity(LOG)
        self.assertEqual(
            t,
            {
                "Tests\\CalcTest::testAdd": ("pass", ""),
                "Tests\\CalcTest::testDivide with data set \"it's\"": ("pass", ""),
                "Tests\\CalcTest::testDivide with data set #1": ("fail", "Failed asserting that 2 is identical to 3.\n"),
                "Tests\\CalcTest::testSkipped": ("skip", "needs redis"),
                "Tests\\CalcTest::testNeverStarted": ("skip", "requires PHP 9"),
                "Tests\\CalcTest::testError": ("fail", "Error: Call to undefined function foo()"),
                "Tests\\CalcTest::testDies": ("crash", "started, never finished"),
            },
        )

    def test_worst_outcome_wins(self):
        start = f"##teamcity[testStarted name='t' locationHint='{LOC}::t' flowId='1']\n"
        fin = "##teamcity[testFinished name='t' flowId='1']\n"
        fail = "##teamcity[testFailed name='t' message='no' flowId='1']\n"
        log = start + fin + start + fail + fin + start + fin
        self.assertEqual(suites.parse_teamcity(log), {"Tests\\CalcTest::t": ("fail", "no")})

    def test_unescape(self):
        self.assertEqual(suites.tc_unescape("a|'b|nc|||[d|]|0x00e9"), "a'b\nc|[d]é")


def results(parts):
    return {"parts": {p: {"exit": e, "timeout": False, "tail": ["Segmentation fault"], "tests": t} for p, (e, t) in parts.items()}}


class Compare(unittest.TestCase):
    def test_ratchet(self):
        php = results({"A": (1, {"a": ["pass", ""], "b": ["pass", ""], "c": ["fail", "x"], "d": ["pass", ""]}),
                       "B": (0, {"e": ["pass", ""], "f": ["pass", ""]})})
        php2 = results({"A": (1, {"a": ["pass", ""], "b": ["pass", ""], "c": ["fail", "x"], "d": ["fail", "y"]}),
                        "B": (0, {"e": ["pass", ""], "f": ["pass", ""]})})
        rhp = results({"A": (139, {"a": ["pass", ""], "b": ["fail", "Error: nope #12"], "c": ["fail", "x"], "d": ["pass", ""]})})
        c = suites.compare("s", [php, php2], rhp, known=["a", "b", "c", "e"], flaky=[])
        self.assertEqual(c["passing"], ["a"])
        self.assertEqual(c["unstable"], ["d"])
        self.assertEqual(c["regressions"], ["b", "e"])
        self.assertEqual(c["env"], ["c"])
        self.assertEqual(c["new"], [])
        self.assertEqual(dict(c["reasons"]), {"fail: Error: nope #N": 1, "part B not run": 2})

    def test_missing_from_a_run_is_not_flaky(self):
        # Part B died in the second php run: its tests are incomplete, not unstable.
        php = results({"A": (0, {"a": ["pass", ""]}), "B": (0, {"b": ["pass", ""], "c": ["fail", "x"]})})
        php2 = results({"A": (0, {"a": ["pass", ""]}), "B": (139, {"c": ["fail", "x"]})})
        php3 = results({"A": (0, {"a": ["fail", "z"]}), "B": (0, {"b": ["pass", ""], "c": ["fail", "x"]})})
        rhp = results({"A": (0, {"a": ["pass", ""]}), "B": (0, {"b": ["pass", ""], "c": ["pass", ""]})})
        c = suites.compare("s", [php, php2, php3], rhp, known=["b"], flaky=[])
        self.assertEqual(c["unstable"], ["a"])
        self.assertEqual(c["incomplete"], ["b"])
        self.assertEqual((c["passing"], c["regressions"], c["env"]), ([], [], []))

    def test_flaky_left_out(self):
        php = results({"A": (0, {"a": ["pass", ""], "b": ["pass", ""]})})
        rhp = results({"A": (0, {"a": ["pass", ""]})})
        c = suites.compare("s", [php], rhp, known=["b"], flaky=["b"])
        self.assertEqual((c["passing"], c["regressions"], c["env"]), (["a"], [], []))
        self.assertEqual(c["new"], ["a"])

    def test_list_roundtrip(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "known.txt"
            keys = ["B::t", "A::t with data set \"x\\ny\"", suites.list_key("A::t2 with data set \"a\nb\"")]
            suites.write_list(p, suites.LIST_HEADER, keys)
            self.assertEqual(suites.read_list(p), sorted(keys))
            self.assertNotIn("a\nb", p.read_text())


class Manifest(unittest.TestCase):
    def test_suites_are_well_formed(self):
        for s in suites.load():
            for k in ("name", "title", "version", "phpunit", "part_args", "parts"):
                self.assertIn(k, s, s["name"])
            self.assertIn("{part}", s["part_args"])
            d = suites.HERE / s["name"]
            for f in s.get("overlay", []):
                self.assertTrue((d / f).is_file(), f"{s['name']}: overlay {f} missing")
            self.assertTrue((d / "composer.lock").is_file(), s["name"])
            if "git" in s:
                self.assertRegex(s["git"]["commit"], r"^[0-9a-f]{40}$")


if __name__ == "__main__":
    unittest.main()
