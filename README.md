<p align="center">
  <a href="https://github.com/hexaquarks/texo/actions/workflows/tests.yml"><img alt="Tests" src="https://img.shields.io/github/actions/workflow/status/hexaquarks/texo/tests.yml?branch=main&amp;label=tests" /></a>
  <a href="https://app.codecov.io/gh/hexaquarks/texo"><img alt="Coverage" src="https://img.shields.io/codecov/c/github/hexaquarks/texo/main" /></a>
  <a href="https://github.com/hexaquarks/texo/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/hexaquarks/texo" /></a>
  <a href="LICENSE"><img alt="MIT License" src="https://img.shields.io/badge/license-MIT-blue" /></a>
</p>

A terminal-native, AI-assisted editor and manager for LaTeX resumes.

Early development. Interfaces and configuration may change.

<p align="center">
  <img src="assets/workspace-example.png" width="1000" alt="Texo in Ghostty with three resume projects, a saved LaTeX document in Vim mode, a short AI exchange, and the complete PDF preview" />
</p>

The screenshot shows the development version; the latest release may differ.

## Features

- AI-assisted LaTeX editing
- OpenAI/Codex, and Anthropic support
- Multiple resume projects and labels
- Standard and Vim editing modes
- Change review and approval mechanism
- Side-by-side PDF preview
- Revision history and restoration
- LaTeX project import
- PDF export

## Install

macOS and Linux, ARM64 and x86-64.

```sh
brew install hexaquarks/tap/texo
# or
cargo install texo
```

Prebuilt binaries: [GitHub Releases](https://github.com/hexaquarks/texo/releases).

> [!IMPORTANT]
> Tectonic and Poppler are required; Homebrew installs both.
> Linux also needs Bubblewrap and D-Bus.
> Linux source builds need the D-Bus development package and `pkg-config`.

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


## Usage

```sh
texo                        # Default or configured workspace
texo ~/resumes              # Open a workspace
texo path/to/main.tex       # Open a TeX file
```

Start by creating or importing a resume from the sidebar. Edit the LaTeX or ask AI for changes, review the PDF, then use Export to choose a filename and save it.

Press `Ctrl+x`, then `?` for shortcuts.

> [!NOTE]
> Change the `Ctrl+x` prefix in `config.toml`, accessible from Settings.

## AI setup

Use `/login` in chat to connect ChatGPT / Codex, OpenAI, or Anthropic.
ChatGPT / Codex requires the Codex CLI; OpenAI and Anthropic use API keys.
Use `/model` to choose a model.

## Contributing

Bug reports, feature requests, and pull requests are all welcome!

[MIT License](LICENSE).
