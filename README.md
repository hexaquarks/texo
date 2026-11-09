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

macOS and Linux, ARM64 and x86-64.

```sh
brew install hexaquarks/tap/texo
# or
cargo install texo --locked
```

Prebuilt binaries: [GitHub Releases](https://github.com/hexaquarks/texo/releases).

> [!IMPORTANT]
> Tectonic and Poppler are required; Homebrew installs both.
> Linux also needs Bubblewrap and D-Bus.
> Linux source builds need the D-Bus development package and `pkg-config`.

## Usage

```sh
texo                       # Default or configured workspace
texo ~/resumes              # Open a workspace
texo path/to/main.tex       # Open a TeX file
```

Press `Ctrl+x`, then `?` for shortcuts.

> [!NOTE]
> Change the `Ctrl+x` prefix in `config.toml`, accessible from Settings.

## AI setup

Use `/login` in chat to connect ChatGPT / Codex, OpenAI, or Anthropic.
ChatGPT / Codex requires the Codex CLI; OpenAI and Anthropic use API keys.
Use `/model` to choose a model.

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

[Terminal tests and screenshots](https://github.com/hexaquarks/texo/actions/workflows/native-terminals.yml).

[MIT License](LICENSE).
