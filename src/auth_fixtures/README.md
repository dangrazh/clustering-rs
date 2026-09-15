# Test-only identity keys

These RSA keys were generated solely for isolated signature-validation tests. They are public test fixtures, not deployment credentials. Rust includes them only under `cfg(test)`. Never register them with Entra or use them for application sessions.
