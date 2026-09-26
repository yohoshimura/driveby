# Security policy

Thank you for helping keep Driveby and the people who use it safe. This page
says what we look after, how to tell us about a problem privately, and what
you can expect from us once you have.

## Supported versions

Only the latest 2.x release receives security fixes. Fixes ship as a new
release, which the built-in updater offers to every installed copy.

| Version        | Supported                    |
| -------------- | ---------------------------- |
| 2.x (latest)   | Yes                          |
| 2.x (older)    | No, update to the latest 2.x |
| 1.x and older  | No                           |

## Reporting a vulnerability

**Please do not open a public issue, discussion or pull request for a
security problem.**

Report it privately through GitHub instead:
**[Report a vulnerability](https://github.com/yohoshimura/driveby/security/advisories/new)**
(the *Security* tab → *Report a vulnerability*). Only the maintainers can see
the report, and we can work on a fix, request a CVE and credit you in the
same place.

A useful report includes:

- the Driveby version and your operating system (Windows, macOS or Linux, and
  version);
- what an attacker can do, and what they need first (for example: another
  local user account, a crafted folder on a USB drive, a position on the
  network between you and GitHub);
- steps to reproduce, or a proof of concept;
- any idea you have for a fix.

## What happens next

| Stage                                  | Our target                       |
| -------------------------------------- | -------------------------------- |
| Acknowledge your report                | within 7 days                    |
| Confirm (or rule out) the problem      | within 14 days                   |
| Share our severity assessment          | within 14 days                   |
| Keep you posted                        | at least every 30 days           |
| Tell you the fix has shipped           | when the release is published    |

Once the problem is confirmed, we aim to release a fix within:

| Severity | Examples                                                           | Fix released within |
| -------- | ------------------------------------------------------------------ | ------------------- |
| Critical | code execution through the updater or a crafted backup source      | 14 days             |
| High     | reading or overwriting files outside what the user chose           | 30 days             |
| Medium   | denial of service of a scheduled backup                            | 90 days             |
| Low      | hardening issues with no direct impact                             | next release        |

These are targets for a small volunteer project, not guarantees. If we are
going to miss one, we will tell you why and when to expect the fix.

## Coordinated disclosure

We ask you to keep the details private until a fixed release is out, or for
90 days from your report, whichever comes first. If we need longer we will
ask, and explain why. When the fix ships we publish a GitHub Security
Advisory that describes the issue and credits you, unless you would rather
stay anonymous.

## Scope

In scope:

- the Driveby application: the Rust backend, the commands it exposes to the
  webview, the frontend and its content security policy, and everything it
  does with files during backup, restore and cleanup (following symlinks or
  junctions, paths escaping a destination, permissions of what it writes);
- the updater: how `latest.json` and update bundles are fetched and how their
  signatures are checked;
- the release pipeline and its artifacts: the GitHub Actions workflows in
  this repository, the installers, `SHA256SUMS`, the SBOMs and attestations.

Out of scope:

- vulnerabilities in a dependency with no Driveby-specific impact (please
  report those upstream; do tell us if a Driveby release ships an affected
  version);
- attacks that need administrator or root access, or full control of the
  user's account, on the machine running Driveby;
- the operating system warning that the installers are not code-signed
  (known; see *Verifying a download* below);
- denial of service, social engineering, and physical attacks;
- reports from automated scanners with no demonstrated impact.

## Safe harbor

We will not pursue or support legal action against anyone who, in good
faith and within this policy, looks for and reports a vulnerability: testing
only against their own installation and their own data, not accessing or
changing other people's data, not degrading anyone's service, and giving us
reasonable time to fix the issue before disclosing it. If in doubt about
whether something is within the policy, ask us first through a private
report.

## Verifying a download

The installers are not yet code-signed by Microsoft or Apple, so the
operating system cannot vouch for them. Releases built by the current release
workflow instead carry:

- **`SHA256SUMS`**, the SHA-256 of every file in the release:

  ```sh
  sha256sum --check --ignore-missing SHA256SUMS
  ```

- **build provenance attestations**, signed through Sigstore by the release
  workflow of this repository, which prove a file was built by that workflow
  from the tagged source:

  ```sh
  gh attestation verify Driveby_<version>_x64-setup.exe --repo yohoshimura/driveby
  ```

- **CycloneDX SBOMs** (`*_cargo.cdx.json`, `*_npm.cdx.json`) listing every
  component compiled into the app.

Updates installed by the built-in updater are verified before they are
applied: each update bundle carries a minisign signature, checked against the
public key compiled into the app (key ID `883B477825312B9`).
