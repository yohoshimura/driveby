# Pending workflow changes

These four files belong in `.github/workflows/`. They are kept here only
because the tool that prepared them may not modify workflow files (GitHub
requires the `workflow` scope for that). To apply them:

```bash
git mv -f docs/devsecops/workflows/ci.yml             .github/workflows/ci.yml
git mv -f docs/devsecops/workflows/release.yml        .github/workflows/release.yml
git mv    docs/devsecops/workflows/codeql.yml         .github/workflows/codeql.yml
git mv    docs/devsecops/workflows/security-audit.yml .github/workflows/security-audit.yml
git rm    docs/devsecops/workflows/README.md
git commit -m "Move the hardened workflows into place" && git push
```

Before the first tagged release with the new `release.yml`, in
**Settings → Environments**, create `release`, restrict it to the tag
pattern `v*`, and move `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` into it from the repository secrets.
Until you do, the repository-level secrets keep working: GitHub creates the
environment on first use and repository secrets stay visible to it.
