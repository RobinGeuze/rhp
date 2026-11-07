interface SessionHandlerInterface {
    public function open(string $path, string $name): bool;
    public function close(): bool;
    public function read(string $id): string|false;
    public function write(string $id, string $data): bool;
    public function destroy(string $id): bool;
    public function gc(int $max_lifetime): int|false;
}
interface SessionIdInterface {
    public function create_sid(): string;
}
interface SessionUpdateTimestampHandlerInterface {
    public function validateId(string $id): bool;
    public function updateTimestamp(string $id, string $data): bool;
}
// mod_user_class.c: the default save handler's methods (__session_handler
// op codes 0..6).
class SessionHandler implements SessionHandlerInterface, SessionIdInterface {
    public function open(string $path, string $name): bool { return __session_handler(0, $path, $name); }
    public function close(): bool { return __session_handler(1); }
    public function read(string $id): string|false { return __session_handler(2, $id); }
    public function write(string $id, string $data): bool { return __session_handler(3, $id, $data); }
    public function destroy(string $id): bool { return __session_handler(4, $id); }
    public function gc(int $max_lifetime): int|false { return __session_handler(5, $max_lifetime); }
    public function create_sid(): string { return __session_handler(6); }
}
