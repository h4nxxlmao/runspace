# Security policy

## Supported versions

Runspace is at v0.1. Security fixes are applied to the latest release on the `main` branch. Older releases may not receive patches.

## Reporting a vulnerability

Please do not open a public GitHub issue for a security-sensitive report.

Report vulnerabilities privately through GitHub's [Security Advisories](https://github.com/h4nxxlmao/runspace/security/advisories/new) or email **support@getsingularity.lol**.

Include:

- The affected version and platform.
- Reproduction steps or a minimal proof of concept.
- The impact you believe is possible.
- Any relevant logs or screenshots.

You should receive an acknowledgement within a few business days. Please allow time for a fix and coordinated disclosure before publishing details.

## Scope

Runspace is a GUI testing sandbox for agents, not a hardened security boundary.

- It isolates the desktop and synthetic input from the host.
- It does **not** isolate the filesystem or network. Applications launched inside a runspace run as your user and can access the same files and network as the Runspace process.
- Windows currently shares the session clipboard across desktops. Linux gives each runspace a private clipboard by default.
- Do not run untrusted code in Runspace and assume the host is protected.

Reports about escaping the intended GUI isolation, unsafe process cleanup, unexpected host input injection, or secrets exposure are in scope.
