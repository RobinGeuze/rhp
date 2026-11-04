<?php
// The date classes on timelib: fixed inputs only (no "now"), no var_dump
// of objects (object ids are not compared), and nothing CI's php reads
// from its system tz database differently (transitions, locations).
date_default_timezone_set('Europe/Amsterdam');
function show($label, $v) { echo $label, ': ', is_string($v) ? $v : json_encode($v), "\n"; }
function err(callable $f) {
    try { $f(); echo "no error\n"; }
    catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
}

echo "== DateTime / DateTimeImmutable ==\n";
$d = new DateTime('2021-03-28 01:30:00');
print_r($d);
show('json', $d);
var_export($d); echo "\n";
print_r((array) $d);
$d->modify('+1 hour');
show('dst gap', $d->format('c T U'));
$i = new DateTimeImmutable('2021-01-31 10:00:00.250000');
show('add P1M', $i->add(new DateInterval('P1M'))->format('Y-m-d H:i:s.u'));
show('sub P1M', $i->sub(new DateInterval('P1M'))->format('Y-m-d'));
show('unchanged', $i->format('Y-m-d'));
foreach (['first day of next month', 'last day of this month', '+1 fortnight', 'tomorrow',
          'next monday', '@86400', 'noon', 'midnight +1 week', 'last sat of July 2008'] as $m) {
    show("modify $m", $i->modify($m)->format('Y-m-d H:i:s T'));
}
show('setDate', (clone $d)->setDate(2020, 13, 35)->format('c'));
show('setISODate', (clone $d)->setISODate(2021, 53, 7)->format('c'));
show('setTime', (clone $d)->setTime(25, 61, 61, 7)->format('c u'));
show('setTimestamp', (clone $d)->setTimestamp(-1)->format('c u'));
show('setMicrosecond', $i->setMicrosecond(42)->format('u'));
show('timezone', $i->setTimezone(new DateTimeZone('Asia/Kolkata'))->format('c e T Z'));
show('offset tz', (new DateTime('2021-06-01 12:00', new DateTimeZone('-03:30')))->format('c e T'));
show('abbr tz', (new DateTime('2021-06-01 12:00 EDT'))->format('c e T I'));
show('getOffset', [$d->getOffset(), $i->getOffset(), (new DateTime('2021-01-01 EST'))->getOffset()]);
show('getTimezone', $d->getTimezone()->getName());
show('createFromTimestamp', DateTime::createFromTimestamp(1234567890.5)->format('c u e'));
show('createFromInterface', get_class(DateTimeImmutable::createFromInterface($d)));
show('createFromMutable', DateTimeImmutable::createFromMutable($d)->format('c'));
show('compare', [$d < $i, $d == clone $d, $d > new DateTime('1999-01-01')]);

echo "== createFromFormat / getLastErrors ==\n";
show('fmt', DateTime::createFromFormat('!d/m/Y', '15/08/2005')->format('c'));
show('fmt U.u', date_create_immutable_from_format('U.u', '1234567890.123456')->format('c u'));
show('fmt bad', DateTime::createFromFormat('Y-m-d', '2021-02-30x'));
print_r(DateTime::getLastErrors());
show('fmt ok', DateTime::createFromFormat('Y-m-d H:i', '2021-02-03 04:05')->format('c'));
show('no errors', DateTime::getLastErrors());
show('date_create bad', date_create('not a date'));
print_r(date_get_last_errors());

echo "== errors ==\n";
err(fn() => new DateTime('foo bar'));
err(fn() => (new DateTime('2021-01-01'))->modify('nonsense'));
show('date_modify', @date_modify(new DateTime('2021-01-01'), 'nonsense'));
err(fn() => new DateTimeZone('Mars/Phobos'));
err(fn() => new DateTimeZone('+99:00'));
err(fn() => new DateInterval('P1X'));
err(fn() => DateInterval::createFromDateString('2021-01-01'));
err(fn() => (new DateTime('2021-01-01'))->sub(DateInterval::createFromDateString('next weekday')));
err(fn() => (new DateTime('2021-01-01'))->setMicrosecond(1000000));
err(fn() => DateTime::createFromTimestamp(INF));
err(fn() => DateTime::__set_state(['date' => 1]));
class Uninit extends DateTime { public function __construct() {} }
err(fn() => (new Uninit)->format('Y'));
err(fn() => (new Uninit) == new DateTime('2021-01-01'));
err(fn() => new DateTimeZone('Europe/Paris') == new DateTimeZone('+01:00'));
err(fn() => new DatePeriod(1, 2, 3, 4, 5));
err(fn() => new DatePeriod(new DateTime('2021-01-01'), new DateInterval('P1D'), 0));
err(fn() => DatePeriod::createFromISO8601String('2012-07-01T00:00:00Z/P7D'));

echo "== DateTimeZone ==\n";
$z = new DateTimeZone('Europe/Amsterdam');
print_r($z);
show('name/offset', [$z->getName(), $z->getOffset(new DateTime('2021-07-01'))]);
show('offset zone', [(new DateTimeZone('+05:30'))->getName(), (new DateTimeZone('EST'))->getName()]);
show('transitions of offset', (new DateTimeZone('+03:00'))->getTransitions());
show('timezone_open', @timezone_open('Mars/Phobos'));
show('equal', new DateTimeZone('Europe/Paris') == new DateTimeZone('Europe/Paris'));
show('identifiers', count(DateTimeZone::listIdentifiers(DateTimeZone::EUROPE)) > 50);
print_r(unserialize(serialize($z)));

echo "== DateInterval ==\n";
$iv = new DateInterval('P1Y2M3DT4H5M6S');
print_r($iv);
show('format', $iv->format('%Y %y %M %m %D %d %H %h %I %i %S %s %F %f %a %R %r %% %x'));
$diff = (new DateTime('2021-03-14 01:00', new DateTimeZone('America/New_York')))
    ->diff(new DateTime('2020-02-29 13:14:15.5', new DateTimeZone('Asia/Tokyo')));
print_r($diff);
show('diff format', $diff->format('%R%a days %h:%I:%S.%F'));
$w = new DateInterval('PT1S');
$w->f = 0.5;
$w->invert = 1;
show('written', $w->format('%R %s %F'));
print_r($w);
$s = DateInterval::createFromDateString('3 days ago');
print_r($s);
show('from string d', $s->d);
show('property_exists', [property_exists($iv, 'y'), isset($iv->days), $iv->nope ?? 'none']);
print_r(unserialize(serialize($iv)));
var_export(DateInterval::__set_state(['y' => 1, 'f' => 0.5, 'days' => 10, 'invert' => 1])); echo "\n";
show('compare', @($iv == new DateInterval('P1D')));

echo "== DatePeriod ==\n";
$p = new DatePeriod(new DateTimeImmutable('2021-01-31'), new DateInterval('P1M'),
    new DateTime('2021-06-01'), DatePeriod::EXCLUDE_START_DATE | DatePeriod::INCLUDE_END_DATE);
foreach ($p as $k => $dt) { echo $k, ' ', get_class($dt), ' ', $dt->format('Y-m-d'), "\n"; }
show('recurrences', [$p->getRecurrences(), $p->recurrences, $p->include_end_date]);
show('current', $p->current->format('c'));
$it = $p->getIterator();
show('iterator', [get_class($it), $it->valid(), $it->key(), $it->current()->format('Y-m-d')]);
$p2 = DatePeriod::createFromISO8601String('R3/2012-07-01T00:00:00Z/P7D');
foreach ($p2 as $dt) { echo $dt->format(DATE_ATOM), "\n"; }
show('getters', [$p2->getEndDate(), $p2->getStartDate()->format('c'), $p2->getDateInterval()->d]);
print_r($p2);
show('round trip', unserialize(serialize($p2)) == $p2);
err(function () use ($p2) { $p2->start = 1; });
err(function () use ($p2) { unset($p2->interval); });
show('isset', [isset($p2->start), isset($p2->end), empty($p2->recurrences)]);

echo "== procedural ==\n";
$o = date_create('2000-02-29 12:00:00');
date_add($o, new DateInterval('P1Y'));
date_time_set($o, 1, 2, 3);
date_date_set($o, 2001, 3, 4);
date_isodate_set($o, 2001, 10, 2);
date_timezone_set($o, timezone_open('UTC'));
show('chain', date_format($o, 'c'));
show('get', [date_timestamp_get($o), date_offset_get($o), timezone_name_get(date_timezone_get($o))]);
show('interval', date_interval_format(date_diff(date_create('2000-01-01'), $o), '%a'));
show('from string', date_interval_format(date_interval_create_from_date_string('1 week'), '%d'));
show('tz offset', timezone_offset_get(timezone_open('Asia/Tokyo'), $o));

echo "== sun ==\n";
print_r(date_sun_info(1626300000, 52.37, 4.89));
print_r(date_sun_info(1626300000, 89.0, 0.0));
show('sunrise', [@date_sunrise(1626300000, SUNFUNCS_RET_STRING, 52.37, 4.89, 90.83, 2),
    @date_sunset(1626300000, SUNFUNCS_RET_STRING, 52.37, 4.89), @date_sunrise(1626300000, SUNFUNCS_RET_TIMESTAMP)]);
