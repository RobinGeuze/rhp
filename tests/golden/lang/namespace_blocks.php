<?php
// Several namespace blocks in one file: declarations, plain, grouped and
// function `use`, relative names, class constants, static calls,
// instanceof, a fully qualified catch and the fall-back to global
// functions.
namespace App\Model;
class User {
    const ROLE = 'admin';
    public static function make($n) { return new self(); }
    public function __construct(public string $name = 'x') {}
    public function role() { return self::ROLE; }
}
interface Named {}
class Admin extends User implements Named {}
function fmt($u) { return $u->name . '/' . $u->role(); }

namespace App2;
use App\Model\User;
use App\Model\{Admin, Named};
use function App\Model\fmt;
$u = new User('bob');
echo fmt($u), "\n";
echo User::ROLE, "\n";
echo User::make(1)->role(), "\n";
$a = new Admin('amy');
var_dump($a instanceof \App\Model\Named);
var_dump($a instanceof User);
echo \App\Model\User::ROLE, "\n";
echo namespace\xyz(), "\n";
function xyz() { return "ns2-xyz"; }
echo strtoupper("hi"), "\n";
try { throw new \RuntimeException("boom"); }
catch (\RuntimeException $e) { echo "caught ", $e->getMessage(), "\n"; }
echo "NS=[", __NAMESPACE__, "]\n";
