<?php
echo http_build_query(['a' => 1, 'b' => 'x y', 'c' => 'ä&=+~']), "\n";
echo http_build_query(['a' => 1, 'b' => 'x y', 'c' => 'ä&=+~'], '', '&', PHP_QUERY_RFC3986), "\n";
echo http_build_query([1, 2, 'k' => 3], 'p_'), "\n";
echo http_build_query(['x' => [1, 2, ['y' => 'z']], 'n' => null, 't' => true, 'f' => false]), "\n";
echo http_build_query([5 => ['a' => [7 => 'q']]], 'num'), "\n";
echo http_build_query(['a' => 1, 'b' => 2], '', '&amp;'), "\n";
echo http_build_query(['a' => 1, 'b' => 2], '', ''), "\n";
echo http_build_query(['a' => 1, 'b' => 2], '', null), "\n";
echo http_build_query(['f' => 1.5, 'g' => 0.1 + 0.2, 'h' => 1e100, 'i' => -0.0]), "\n";
echo http_build_query(['k y' => ['s p' => 'v']]), "\n";
echo http_build_query(['k y' => ['s p' => 'v']], '', '&', PHP_QUERY_RFC3986), "\n";
var_dump(http_build_query([]));
var_dump(http_build_query(['a' => null]));
var_dump(http_build_query(['a' => []]));

#[AllowDynamicProperties]
class P {
    public $pub = 'p';
    protected $prot = 'q';
    private $priv = 'r';
    public $arr = ['x' => 1];
    public $nul = null;
}
$o = new P();
$o->dyn = 'd';
echo http_build_query($o), "\n";
echo http_build_query(['obj' => $o]), "\n";
$s = new stdClass();
$s->a = 'b';
$s->list = [1, 2];
echo http_build_query($s), "\n";

enum Suit: string {
    case Hearts = 'H';
}
enum Plain {
    case One;
}
echo http_build_query(['s' => Suit::Hearts]), "\n";
try {
    http_build_query(['s' => Plain::One]);
} catch (ValueError $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
try {
    http_build_query(Suit::Hearts);
} catch (TypeError $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
try {
    http_build_query("str");
} catch (TypeError $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}

// parse_str
$cases = [
    'a=1&b=2', 'a[]=1&a[]=2', 'a[x]=1&a[y][z]=2', 'a b.c=1', 'a[b.c d]=1',
    'a[b][c=1', 'a[b=1', 'a[ ]=1', 'a[ x]=1', 'a[b]c=1', '%20a=1', 'a%00b=1',
    'a=1%002', '1=x&01=y&-1=z', 'a=1&a[x]=2', 'a[x]=1&a=2', '__Host-x=1',
    'x[__Host.x]=1', 'a[][b]=1&a[][b]=2', 'a[[x]]=1', 'a][=1', '=1',
    'a&b&&c=', 'a[b[c]=1', 'a[1][2]=x&a[1][3]=y', 'q=a+b%2Bc%zz%4', '',
    'arr[]=1&arr[5]=2&arr[]=3', 'a.b[c.d]=1', ' x=1', "a=1\0b=2",
];
foreach ($cases as $q) {
    parse_str($q, $r);
    echo json_encode($q), ' => ', json_encode($r), "\n";
}
$r = 'not an array';
parse_str('k=v', $r);
var_dump($r);
