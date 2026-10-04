# TLS fixtures

`ca.pem` is a throwaway self-signed EC certificate (`CN=oxikube-test-ca`, 100 years) used only by
`src/pool/security_tests.rs` as certificate-authority data / file. It has no private key anywhere
in the repository and trusts nothing.
