<?php
// bindParam without a type binds text; query() holds a hidden
// SQLite3Stmt object (object handles); a PDO subclass's __call answers
// before PDO's driver methods; IteratorIterator's $class checks.
$db = new SQLite3(':memory:');
$db->exec('create table t(a)');
$st = $db->prepare('insert into t values(:a)');
$v = 1;
$st->bindParam(':a', $v);
$st->execute();
$st->bindValue(':a', 2);
$st->execute();
$st->bindParam(':a', $v, SQLITE3_INTEGER);
$st->execute();
var_dump($st->getSQL(true));
$res = $db->query('select a, typeof(a) from t');
while ($row = $res->fetchArray(SQLITE3_NUM)) {
    echo $row[0], ' ', $row[1], "\n";
}
$res = $db->query('select 1');
var_dump($res, spl_object_id(new stdClass));

class P extends PDO
{
    public function __call($n, $a)
    {
        echo "__call $n\n";
    }
}
$p = new P('sqlite::memory:');
$p->sqliteCreateFunction('f', 'strlen');
var_dump(method_exists($p, 'sqliteCreateFunction'));

class A implements IteratorAggregate
{
    public function getIterator(): Iterator
    {
        echo "A\n";
        return new ArrayIterator([1]);
    }
}
class B extends A
{
    public function getIterator(): Iterator
    {
        echo "B\n";
        return new ArrayIterator([2]);
    }
}
foreach (['A', 'B', 'IteratorAggregate', 'Traversable', 'Nope'] as $c) {
    try {
        foreach (new IteratorIterator(new B(), $c) as $x) {
            var_dump($x);
        }
    } catch (LogicException $e) {
        echo $e->getMessage(), "\n";
    }
}
