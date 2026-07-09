# Security Policy

## Supported Versions

Currently, the following versions of kaji are supported with security updates:

| Version | Status                |
| ------- | --------------------- |
| 2603.x  | :white_check_mark: Supported   |

## Reporting a Vulnerability

If you discover a potential security vulnerability in `kaji`, please do **not** open a public issue. Instead, report it privately to the maintainers:

- Fabio Falcinelli: [fabio.falcinelli@gmail.com](mailto:fabio.falcinelli@gmail.com)

We aim to acknowledge receipt of your report as soon as possible (typically within a few business days). Please note that while we take security seriously, we are a community-maintained project and cannot guarantee a specific resolution timeframe. We will provide updates as we investigate the issue and work toward a fix.

### What to Include in a Report

To help us address the issue quickly, please include:
- A clear description of the vulnerability.
- A minimal reproducible example (PoC) if possible.
- Any potential impact or exploitation scenarios.

## Security Best Practices for kaji Users

`kaji` interacts with the Keycloak Admin API and manages sensitive configuration data. To ensure your usage remains secure:

1.  **Principle of Least Privilege**: Ensure that the Keycloak client or user credentials used by `kaji` only have the minimum necessary permissions to manage the target realm.
2.  **Secret Management**: `kaji` automatically masks detected secrets in local YAML configurations and stores them in a separate `.secrets` file. Do **not** commit `.secrets` files to version control.
3.  **Environment Variables**: Protect environment variables containing Keycloak credentials (e.g., `KEYCLOAK_CLIENT_SECRET`) used by `kaji`.
4.  **Keep kaji Updated**: Ensure you are using the latest version of `kaji` to benefit from upstream security fixes.

## Disclosure Policy

We follow a responsible disclosure policy:
1.  Acknowledge the report.
2.  Investigate and confirm the vulnerability.
3.  Work on a fix.
4.  Release a new version with the fix.
5.  Publicly disclose the vulnerability (e.g., via GitHub Security Advisories) after a fix is available and users have had time to update.
