`localhost.der` and `test-key.der` are a self-signed certificate and its public
fixture key, generated solely for local HTTPS integration tests. They contain
no production credentials. The tests verify default rejection and explicit
per-client configuration changes without external network access.
