# Packaging templates

These templates are ready to copy into the matching community repositories:

| Directory | Target | Publication |
|-----------|--------|-------------|
| `homebrew/tlog.rb` | Homebrew (macOS / Linux) | Add to a Homebrew tap or `homebrew-core` |
| `scoop/tlog.json` | Scoop (Windows) | Add to a Scoop bucket manifest |
| `aur/PKGBUILD` | Arch Linux (AUR) | Submit as `tlog-bin` |

Each template pins version `1.4.0` and points at the standard archive names
created by `.github/workflows/release.yml`:

- `tlog-x86_64-unknown-linux-gnu.tar.gz`
- `tlog-x86_64-pc-windows-msvc.zip`
- `tlog-x86_64-apple-darwin.tar.gz`
- `tlog-aarch64-apple-darwin.tar.gz`

Before publishing, replace every `REPLACE_WITH_*_SHA256` with the matching
digest from the release's `SHA256SUMS` file. `cargo-binstall` can consume the
same archive names directly from GitHub Releases, no packaging repository
required:

```bash
cargo binstall tlog
```
