# Exec client-certificate fixtures

`client.crt` / `client.key` are a throwaway self-signed EC pair (`CN=oxikube-test-exec-client`, 100
years) that `auth_exec_cert.rs` hands out from a fake exec credential plugin
(`clientCertificateData` / `clientKeyData`). The key protects nothing: no cluster, server or
account trusts this certificate. Regenerate with

    openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
      -keyout client.key -out client.crt -days 36500 -subj "/CN=oxikube-test-exec-client"
