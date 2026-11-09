
final class Socket {
    public function __construct() { throw new Error("Cannot directly construct Socket, use socket_create() instead"); }
}

final class AddressInfo {
    public function __construct() { throw new Error("Cannot directly construct AddressInfo, use socket_addrinfo_lookup() instead"); }
}
