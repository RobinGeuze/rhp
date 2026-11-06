class DateError extends Error {}
class DateObjectError extends DateError {}
class DateRangeError extends DateError {}
class DateException extends Exception {}
class DateInvalidTimeZoneException extends DateException {}
class DateInvalidOperationException extends DateException {}
class DateMalformedStringException extends DateException {}
class DateMalformedIntervalStringException extends DateException {}
class DateMalformedPeriodStringException extends DateException {}

interface DateTimeInterface {
    const ATOM = "Y-m-d\\TH:i:sP";
    const COOKIE = "l, d-M-Y H:i:s T";
    const ISO8601 = "Y-m-d\\TH:i:sO";
    const ISO8601_EXPANDED = "X-m-d\\TH:i:sP";
    const RFC822 = "D, d M y H:i:s O";
    const RFC850 = "l, d-M-y H:i:s T";
    const RFC1036 = "D, d M y H:i:s O";
    const RFC1123 = "D, d M Y H:i:s O";
    const RFC7231 = "D, d M Y H:i:s \\G\\M\\T";
    const RFC2822 = "D, d M Y H:i:s O";
    const RFC3339 = "Y-m-d\\TH:i:sP";
    const RFC3339_EXTENDED = "Y-m-d\\TH:i:s.vP";
    const RSS = "D, d M Y H:i:s O";
    const W3C = "Y-m-d\\TH:i:sP";
    public function format(string $format);
    public function getTimezone();
    public function getOffset();
    public function getTimestamp();
    public function getMicrosecond(): int;
    public function diff(DateTimeInterface $targetObject, bool $absolute = false);
    public function __wakeup();
    public function __serialize(): array;
    public function __unserialize(array $data): void;
}

class DateTime implements DateTimeInterface {
    public function __construct(string $datetime = "now", ?DateTimeZone $timezone = null) { __date('construct', $this, $datetime, $timezone); }
    public function __serialize(): array { return __date('__serialize', $this); }
    public function __unserialize(array $data): void { __date('__unserialize', $this, $data); }
    public function __wakeup() { __date('__wakeup', $this); }
    public static function __set_state(array $array) { return __date('__set_state', 'DateTime', $array); }
    public static function createFromImmutable(DateTimeImmutable $object) { return __date('createFrom', static::class, $object); }
    public static function createFromInterface(DateTimeInterface $object) { return __date('createFrom', static::class, $object); }
    public static function createFromFormat(string $format, string $datetime, ?DateTimeZone $timezone = null) { return __date('createFromFormat', static::class, $format, $datetime, $timezone, 'DateTime::createFromFormat'); }
    public static function createFromTimestamp(int|float $timestamp): static { return __date('createFromTimestamp', static::class, $timestamp); }
    public static function getLastErrors() { return __date('getLastErrors'); }
    public function format(string $format) { return __date('format', $this, $format); }
    public function modify(string $modifier) { return __date('modify', $this, $modifier, 'DateTime::modify'); }
    public function add(DateInterval $interval) { return __date('add', $this, $interval); }
    public function sub(DateInterval $interval) { return __date('sub', $this, $interval, 'DateTime::sub'); }
    public function getTimezone() { return __date('getTimezone', $this); }
    public function setTimezone(DateTimeZone $timezone) { return __date('setTimezone', $this, $timezone); }
    public function getOffset() { return __date('getOffset', $this); }
    public function getMicrosecond(): int { return __date('getMicrosecond', $this); }
    public function setTime(int $hour, int $minute, int $second = 0, int $microsecond = 0) { return __date('setTime', $this, $hour, $minute, $second, $microsecond); }
    public function setDate(int $year, int $month, int $day) { return __date('setDate', $this, $year, $month, $day); }
    public function setISODate(int $year, int $week, int $dayOfWeek = 1) { return __date('setISODate', $this, $year, $week, $dayOfWeek); }
    public function setTimestamp(int $timestamp) { return __date('setTimestamp', $this, $timestamp); }
    public function setMicrosecond(int $microsecond): static { return __date('setMicrosecond', $this, $microsecond); }
    public function getTimestamp() { return __date('getTimestamp', $this); }
    public function diff(DateTimeInterface $targetObject, bool $absolute = false) { return __date('diff', $this, $targetObject, $absolute); }
}

class DateTimeImmutable implements DateTimeInterface {
    public function __construct(string $datetime = "now", ?DateTimeZone $timezone = null) { __date('construct', $this, $datetime, $timezone); }
    public function __serialize(): array { return __date('__serialize', $this); }
    public function __unserialize(array $data): void { __date('__unserialize', $this, $data); }
    public function __wakeup() { __date('__wakeup', $this); }
    public static function __set_state(array $array) { return __date('__set_state', 'DateTimeImmutable', $array); }
    public static function createFromFormat(string $format, string $datetime, ?DateTimeZone $timezone = null) { return __date('createFromFormat', static::class, $format, $datetime, $timezone, 'DateTimeImmutable::createFromFormat'); }
    public static function createFromTimestamp(int|float $timestamp): static { return __date('createFromTimestamp', static::class, $timestamp); }
    public static function getLastErrors() { return __date('getLastErrors'); }
    public function format(string $format) { return __date('format', $this, $format); }
    public function getTimezone() { return __date('getTimezone', $this); }
    public function getOffset() { return __date('getOffset', $this); }
    public function getTimestamp() { return __date('getTimestamp', $this); }
    public function getMicrosecond(): int { return __date('getMicrosecond', $this); }
    public function diff(DateTimeInterface $targetObject, bool $absolute = false) { return __date('diff', $this, $targetObject, $absolute); }
    #[\NoDiscard(message: "as DateTimeImmutable::modify() does not modify the object itself")]
    public function modify(string $modifier) { return __date('modify', clone $this, $modifier, 'DateTimeImmutable::modify'); }
    #[\NoDiscard(message: "as DateTimeImmutable::add() does not modify the object itself")]
    public function add(DateInterval $interval) { return __date('add', clone $this, $interval); }
    #[\NoDiscard(message: "as DateTimeImmutable::sub() does not modify the object itself")]
    public function sub(DateInterval $interval) { return __date('sub', clone $this, $interval, 'DateTimeImmutable::sub'); }
    #[\NoDiscard(message: "as DateTimeImmutable::setTimezone() does not modify the object itself")]
    public function setTimezone(DateTimeZone $timezone) { return __date('setTimezone', clone $this, $timezone); }
    #[\NoDiscard(message: "as DateTimeImmutable::setTime() does not modify the object itself")]
    public function setTime(int $hour, int $minute, int $second = 0, int $microsecond = 0) { return __date('setTime', clone $this, $hour, $minute, $second, $microsecond); }
    #[\NoDiscard(message: "as DateTimeImmutable::setDate() does not modify the object itself")]
    public function setDate(int $year, int $month, int $day) { return __date('setDate', clone $this, $year, $month, $day); }
    #[\NoDiscard(message: "as DateTimeImmutable::setISODate() does not modify the object itself")]
    public function setISODate(int $year, int $week, int $dayOfWeek = 1) { return __date('setISODate', clone $this, $year, $week, $dayOfWeek); }
    #[\NoDiscard(message: "as DateTimeImmutable::setTimestamp() does not modify the object itself")]
    public function setTimestamp(int $timestamp) { return __date('setTimestamp', clone $this, $timestamp); }
    #[\NoDiscard(message: "as DateTimeImmutable::setMicrosecond() does not modify the object itself")]
    public function setMicrosecond(int $microsecond): static { __date('usCheck', $this, $microsecond); return __date('setMicrosecond', clone $this, $microsecond); }
    public static function createFromMutable(DateTime $object) { return __date('createFrom', static::class, $object); }
    public static function createFromInterface(DateTimeInterface $object) { return __date('createFrom', static::class, $object); }
}

class DateTimeZone {
    const AFRICA = 1;
    const AMERICA = 2;
    const ANTARCTICA = 4;
    const ARCTIC = 8;
    const ASIA = 16;
    const ATLANTIC = 32;
    const AUSTRALIA = 64;
    const EUROPE = 128;
    const INDIAN = 256;
    const PACIFIC = 512;
    const UTC = 1024;
    const ALL = 2047;
    const ALL_WITH_BC = 4095;
    const PER_COUNTRY = 4096;
    public function __construct(string $timezone) { __date('tz_construct', $this, $timezone); }
    public function getName() { return __date('tz_name', $this); }
    public function getOffset(DateTimeInterface $datetime) { return __date('tz_offset', $this, $datetime); }
    public function getTransitions(int $timestampBegin = PHP_INT_MIN, int $timestampEnd = 2147483647) { return __date('tz_transitions', $this, $timestampBegin, $timestampEnd); }
    public function getLocation() { return __date('tz_location', $this); }
    public static function listAbbreviations() { return timezone_abbreviations_list(); }
    public static function listIdentifiers(int $timezoneGroup = DateTimeZone::ALL, ?string $countryCode = null) { return __date('tz_identifiers', $timezoneGroup, $countryCode); }
    public function __serialize(): array { return __date('tz_serialize', $this); }
    public function __unserialize(array $data): void { __date('tz_unserialize', $this, $data); }
    public function __wakeup() { __date('tz_wakeup', $this); }
    public static function __set_state(array $array) { return __date('tz_set_state', $array); }
}

class DateInterval {
    public function __construct(string $duration) { __date('iv_construct', $this, $duration); }
    public static function createFromDateString(string $datetime) { return __date('iv_from_string', $datetime, true); }
    public function format(string $format) { return __date('iv_format', $this, $format); }
    public function __serialize(): array { return __date('iv_serialize', $this); }
    public function __unserialize(array $data): void { __date('iv_unserialize', $this, $data); }
    public function __wakeup() { __date('iv_wakeup', $this); }
    public static function __set_state(array $array) { return __date('iv_set_state', $array); }
}

class DatePeriod implements IteratorAggregate {
    const EXCLUDE_START_DATE = 1;
    const INCLUDE_END_DATE = 2;
    public static function createFromISO8601String(string $specification, int $options = 0): static { return __date('dp_from_iso', static::class, $specification, $options); }
    public function __construct($start = null, $interval = null, $end = null, $options = null) { __date('dp_construct', $this, func_get_args()); }
    public function getStartDate() { return __date('dp_start', $this); }
    public function getEndDate() { return __date('dp_end', $this); }
    public function getDateInterval() { return __date('dp_interval', $this); }
    public function getRecurrences() { return __date('dp_recurrences', $this); }
    public function __serialize(): array { return __date('dp_serialize', $this); }
    public function __unserialize(array $data): void { __date('dp_unserialize', $this, $data); }
    public function __wakeup() { __date('dp_wakeup', $this); }
    public static function __set_state(array $array) { return __date('dp_set_state', $array); }
    public function getIterator(): Iterator { return __date('dp_iterator', $this); }
    public function __get($name) {
        if (__date('dp_internal', $name)) { return __date('dp_get', $this, $name); }
        return $this->$name;
    }
    public function __set($name, $value) {
        __date('dp_set', $this, $name);
        $this->$name = $value;
    }
    public function __isset($name) {
        $r = __date('dp_isset', $this, $name);
        if ($r === null) { return isset($this->$name); }
        return $r;
    }
    public function __unset($name) {
        __date('dp_unset', $this, $name);
        unset($this->$name);
    }
}

function date_create(string $datetime = "now", ?DateTimeZone $timezone = null) { return __date('date_create', 'DateTime', $datetime, $timezone); }
function date_create_immutable(string $datetime = "now", ?DateTimeZone $timezone = null) { return __date('date_create', 'DateTimeImmutable', $datetime, $timezone); }
function date_create_from_format(string $format, string $datetime, ?DateTimeZone $timezone = null) { return __date('createFromFormat', 'DateTime', $format, $datetime, $timezone, 'date_create_from_format'); }
function date_create_immutable_from_format(string $format, string $datetime, ?DateTimeZone $timezone = null) { return __date('createFromFormat', 'DateTimeImmutable', $format, $datetime, $timezone, 'date_create_immutable_from_format'); }
function date_get_last_errors() { return __date('getLastErrors'); }
function date_format(DateTimeInterface $object, string $format) { return __date('format', $object, $format); }
function date_modify(DateTime $object, string $modifier) { return __date('modify', $object, $modifier, 'date_modify'); }
function date_add(DateTime $object, DateInterval $interval) { return __date('add', $object, $interval); }
function date_sub(DateTime $object, DateInterval $interval) { return __date('sub', $object, $interval, 'date_sub'); }
function date_timezone_get(DateTimeInterface $object) { return __date('getTimezone', $object); }
function date_timezone_set(DateTime $object, DateTimeZone $timezone) { return __date('setTimezone', $object, $timezone); }
function date_offset_get(DateTimeInterface $object) { return __date('getOffset', $object); }
function date_diff(DateTimeInterface $baseObject, DateTimeInterface $targetObject, bool $absolute = false) { return __date('diff', $baseObject, $targetObject, $absolute); }
function date_time_set(DateTime $object, int $hour, int $minute, int $second = 0, int $microsecond = 0) { return __date('setTime', $object, $hour, $minute, $second, $microsecond); }
function date_date_set(DateTime $object, int $year, int $month, int $day) { return __date('setDate', $object, $year, $month, $day); }
function date_isodate_set(DateTime $object, int $year, int $week, int $dayOfWeek = 1) { return __date('setISODate', $object, $year, $week, $dayOfWeek); }
function date_timestamp_set(DateTime $object, int $timestamp) { return __date('setTimestamp', $object, $timestamp); }
function date_timestamp_get(DateTimeInterface $object) { return __date('getTimestamp', $object); }
function timezone_open(string $timezone) { return __date('tz_open', $timezone); }
function timezone_name_get(DateTimeZone $object) { return __date('tz_name', $object); }
function timezone_offset_get(DateTimeZone $object, DateTimeInterface $datetime) { return __date('tz_offset', $object, $datetime); }
function timezone_transitions_get(DateTimeZone $object, int $timestampBegin = PHP_INT_MIN, int $timestampEnd = 2147483647) { return __date('tz_transitions', $object, $timestampBegin, $timestampEnd); }
function timezone_location_get(DateTimeZone $object) { return __date('tz_location', $object); }
function date_interval_create_from_date_string(string $datetime) { return __date('iv_from_string', $datetime, false); }
function date_interval_format(DateInterval $object, string $format) { return __date('iv_format', $object, $format); }
