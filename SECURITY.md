# Security Policy

## Supported Versions

Leteo is pre-release software. Security fixes are applied to the latest code on
the `main` branch until the first stable release establishes a version policy.

## Reporting A Vulnerability

Use GitHub's private security advisory feature for the Leteo repository. Do not
open a public issue for vulnerabilities involving authentication bypass,
secret disclosure, database corruption, path traversal, or remote code
execution.

Include the affected revision, operating system, reproduction steps, expected
behavior, actual behavior, and whether the issue affects local or cloud mode.

## Deployment Requirements

- Keep the local HTTP server bound to loopback.
- Put cloud deployments behind TLS termination.
- Use independent random values of at least 32 bytes for dashboard, token
  pepper, sync token, and admin token secrets.
- Restrict legacy tokens with `LETEO_CLOUD_ALLOWED_PROJECTS`.
- Use a dedicated PostgreSQL role and database with least privilege.
- Back up the SQLite database before importing untrusted archives or upgrading
  production data.
