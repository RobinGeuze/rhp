
final class OpenSSLCertificate {
    public function __construct() { throw new Error("Cannot directly construct OpenSSLCertificate, use openssl_x509_read() instead"); }
}
final class OpenSSLCertificateSigningRequest {
    public function __construct() { throw new Error("Cannot directly construct OpenSSLCertificateSigningRequest, use openssl_csr_new() instead"); }
}
final class OpenSSLAsymmetricKey {
    public function __construct() { throw new Error("Cannot directly construct OpenSSLAsymmetricKey, use openssl_pkey_new() instead"); }
}
