<p align="center">
  <a href="https://github.com/hexaquarks/texo/actions/workflows/tests.yml"><img alt="Tests" src="https://img.shields.io/github/actions/workflow/status/hexaquarks/texo/tests.yml?branch=main&amp;label=tests" /></a>
  <a href="https://app.codecov.io/gh/hexaquarks/texo"><img alt="Coverage" src="https://img.shields.io/codecov/c/github/hexaquarks/texo/main" /></a>
  <a href="Cargo.toml"><img alt="Rust edition: 2024" src="https://img.shields.io/badge/Rust-2024-DEA584?logo=rust" /></a>
  <a href="Cargo.toml"><img alt="Ratatui version: 0.30.2" src="https://img.shields.io/badge/Ratatui-0.30.2-blue" /></a>
</p>

A terminal-native, AI-powered resume harness for creating, editing, and managing LaTeX resumes.

Work in progress

<p align="center">
  <img src="assets/workspace-example.png" width="1000" alt="Texo running in Ghostty with labeled resume projects, revision history, a proposed LaTeX edit, AI review controls, and a PDF preview" />
</p>

## Install

Releases target macOS and Linux on ARM64 and x86-64.

> [!NOTE]
> Homebrew and crates.io installs become available after the first release is published.

### Homebrew

```sh
brew install hexaquarks/tap/texo
```

Homebrew also installs Tectonic and Poppler.

### Prebuilt binaries

Download your platform's archive and SHA-256 checksum from
[GitHub Releases](https://github.com/hexaquarks/texo/releases), or use the installer:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/hexaquarks/texo/releases/latest/download/texo-installer.sh | sh
```

> [!TIP]
> The installer places `texo` in `~/.local/bin`. Add that directory to your `PATH`.

### Cargo

With a current stable [Rust toolchain](https://rustup.rs):

```sh
cargo install texo --locked
```

To build a checkout instead, run `cargo install --path . --locked`.

> [!IMPORTANT]
> Binary and Cargo installs require [Tectonic](https://tectonic-typesetting.github.io/)
> and [Poppler](https://poppler.freedesktop.org/) on `PATH` for compilation and PDF previews.
> All Linux installs also need Bubblewrap, D-Bus, and a Secret Service keyring for credentials.
> Building from source on Linux needs the D-Bus development package and `pkg-config`.
> Native Windows compilation is unsupported.

## Compatibility

| Terminal | Compatibility | Testing | Limitations |
| --- | --- | --- | --- |
| Kitty | ✅ Works | CI | — |
| Ghostty | ✅ Works | CI | — |
| WezTerm | ✅ Works | CI | — |
| Alacritty | ⚠️ Limited | CI | External PDF viewer required |
| GNOME Terminal | ⚠️ Limited | CI | External PDF viewer required |
| iTerm2 | ❓ Needs testing | Pending | — |
| Apple Terminal | ⚠️ Limited | Manual preview | External PDF viewer required |
| Windows Terminal (WSL) | ❓ Needs testing | Pending | — |

CI runs a suite of terminal smoke tests and captures screenshots for visual review on Linux x86-64 (X11).
Manual preview checks cover macOS. Full checks on macOS, WSL, and Linux Wayland are pending.

See [test results and screenshots](https://github.com/hexaquarks/texo/actions/workflows/native-terminals.yml).

App logic and PDF compilation are also tested on Intel and Apple Silicon Macs.

## Releases

Versions follow [SemVer](https://semver.org/). Update `Cargo.toml` and `Cargo.lock` in a PR,
merge it, then push the matching `vX.Y.Z` tag from that commit. Tags with a prerelease
suffix publish GitHub prereleases only.

[cargo-dist](https://opensource.axo.dev/cargo-dist/) builds all four targets, creates checksums
and installers, and publishes tagged releases. PRs build the same artifacts without publishing.
The release checks include tests and a crates.io packaging dry run.

> [!IMPORTANT]
> Before the first release, create `hexaquarks/homebrew-tap` and configure repository secrets:
> `HOMEBREW_TAP_TOKEN` with write access to that tap, and `CARGO_REGISTRY_TOKEN` with permission
> to publish `texo` on crates.io. No release credentials are needed for PR builds.

Release configuration lives in `dist-workspace.toml`. After changing it, regenerate the workflow
with cargo-dist 0.33.0: `dist generate`.
