<?php
function a() { try { try { return "r"; } finally { echo "inner\n"; } } finally { echo "outer\n"; } }
echo a(), "\n";
function b() { for ($i=0;$i<3;$i++) { try { try { if ($i==1) break; echo "i$i\n"; } finally { echo "in$i\n"; } } finally { echo "out$i\n"; } } echo "after\n"; }
b();
function c() { foreach ([1,2,3] as $v) { try { if ($v==2) continue; echo "v$v\n"; } finally { echo "f$v\n"; } } }
c();
function d() { try { return 1; } finally { try { echo "x\n"; } finally { echo "y\n"; } } }
echo d(), "\n";
function e() { try { return 1; } finally { return 2; } }
echo e(), "\n";
function f() { while (true) { try { while (true) { try { break 2; } finally { echo "a\n"; } } } finally { echo "b\n"; } } echo "done\n"; }
f();
function g() { try { throw new Exception("x"); } finally { return "swallowed"; } }
echo g(), "\n";
function h() { for ($i=0;$i<2;$i++) { try { throw new Exception("e$i"); } catch (Exception $e) { continue; } finally { echo "fin$i\n"; } } echo "end\n"; }
h();
function k() { try { try { throw new Exception("a"); } finally { echo "f1\n"; } } catch (Exception $e) { echo "c ", $e->getMessage(), "\n"; return "k"; } finally { echo "f2\n"; } }
echo k(), "\n";
function l() { switch (1) { case 1: try { break; } finally { echo "sw\n"; } } echo "lend\n"; }
l();
