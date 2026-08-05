# Security policy

## Reporting a vulnerability

Please report security issues privately through GitHub's security-advisory
feature for `maddiedreese/monitorthesituation`. Do not open a public issue that
contains credentials, private stream URLs, or reproduction steps against a
camera you do not own.

## Configuration safety

- Store secrets in environment variables, not YAML files.
- Treat configurations received from other people as untrusted.
- Only connect to sources you recognize and are authorized to access.
- Camera URLs can reveal internal hostnames and credentials; redact them from
  logs, screenshots, issues, and terminal recordings.

The application invokes FFmpeg directly without a shell, so URL contents are
not interpreted as shell commands. HTTP header names and values reject embedded
newlines to prevent header injection.

