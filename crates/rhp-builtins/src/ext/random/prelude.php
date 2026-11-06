namespace Random {
    interface Engine {
        public function generate(): string;
    }
    interface CryptoSafeEngine extends Engine {
    }
    class RandomError extends \Error {
    }
    class BrokenRandomEngineError extends RandomError {
    }
    class RandomException extends \Exception {
    }
    enum IntervalBoundary {
        case ClosedOpen;
        case ClosedClosed;
        case OpenClosed;
        case OpenOpen;
    }
    final class Randomizer {
        public readonly Engine $engine;
        public function __construct(?Engine $engine = null) {
            $this->engine = $engine ?? new Engine\Secure();
            __randomizer('bind', $this, $this->engine);
        }
        public function nextInt(): int { return __randomizer('nextInt', $this); }
        public function nextFloat(): float { return __randomizer('nextFloat', $this); }
        public function getFloat(float $min, float $max, IntervalBoundary $boundary = IntervalBoundary::ClosedOpen): float {
            return __randomizer('getFloat', $this, $min, $max, $boundary->name);
        }
        public function getInt(int $min, int $max): int { return __randomizer('getInt', $this, $min, $max); }
        public function getBytes(int $length): string { return __randomizer('getBytes', $this, $length); }
        public function getBytesFromString(string $string, int $length): string {
            return __randomizer('getBytesFromString', $this, $string, $length);
        }
        public function shuffleArray(array $array): array { return __randomizer('shuffleArray', $this, $array); }
        public function shuffleBytes(string $bytes): string { return __randomizer('shuffleBytes', $this, $bytes); }
        public function pickArrayKeys(array $array, int $num): array {
            return __randomizer('pickArrayKeys', $this, $array, $num);
        }
        public function __serialize(): array { return [get_object_vars($this)]; }
        public function __unserialize(array $data): void {
            if (count($data) !== 1 || !array_key_exists(0, $data) || !is_array($data[0])) {
                throw new \Exception('Invalid serialization data for Random\Randomizer object');
            }
            try {
                foreach ($data[0] as $k => $v) {
                    if ($k !== 'engine') {
                        // object_properties_load: the class allows no dynamic properties
                        throw new \Error("Cannot create dynamic property Random\\Randomizer::\$$k");
                    }
                    $this->$k = $v;
                }
            } catch (\Throwable $e) {
                throw new \Exception('Invalid serialization data for Random\Randomizer object', 0, $e);
            }
            if (!(($this->engine ?? null) instanceof Engine)) {
                throw new \Exception('Invalid serialization data for Random\Randomizer object');
            }
            __randomizer('bind', $this, $this->engine);
        }
    }
}
namespace Random\Engine {
    final class Mt19937 implements \Random\Engine {
        public function __construct(int|null $seed = null, int $mode = MT_RAND_MT19937) {
            __random_engine('Mt19937::__construct', $this, $seed, $mode);
        }
        public function generate(): string { return __random_engine('generate', $this); }
        public function __serialize(): array { return [get_object_vars($this), __random_engine('state', $this)]; }
        public function __unserialize(array $data): void { __random_engine('unserialize', $this, $data); }
        public function __debugInfo(): array { return get_object_vars($this) + ['__states' => __random_engine('state', $this)]; }
    }
    final class PcgOneseq128XslRr64 implements \Random\Engine {
        public function __construct(string|int|null $seed = null) {
            __random_engine('PcgOneseq128XslRr64::__construct', $this, $seed);
        }
        public function generate(): string { return __random_engine('generate', $this); }
        public function jump(int $advance): void { __random_engine('jump', $this, $advance); }
        public function __serialize(): array { return [get_object_vars($this), __random_engine('state', $this)]; }
        public function __unserialize(array $data): void { __random_engine('unserialize', $this, $data); }
        public function __debugInfo(): array { return get_object_vars($this) + ['__states' => __random_engine('state', $this)]; }
    }
    final class Xoshiro256StarStar implements \Random\Engine {
        public function __construct(string|int|null $seed = null) {
            __random_engine('Xoshiro256StarStar::__construct', $this, $seed);
        }
        public function generate(): string { return __random_engine('generate', $this); }
        public function jump(): void { __random_engine('jump', $this); }
        public function jumpLong(): void { __random_engine('jumpLong', $this); }
        public function __serialize(): array { return [get_object_vars($this), __random_engine('state', $this)]; }
        public function __unserialize(array $data): void { __random_engine('unserialize', $this, $data); }
        public function __debugInfo(): array { return get_object_vars($this) + ['__states' => __random_engine('state', $this)]; }
    }
    final class Secure implements \Random\CryptoSafeEngine {
        public function generate(): string { return __random_engine('generate', $this); }
    }
}
