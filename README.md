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

## Compatibility

| Terminal | Texo compatibility | Testing | Limitations |
| --- | --- | --- | --- |
| Kitty | ✅ Works | CI | — |
| Ghostty | ✅ Works | CI | — |
| WezTerm | ✅ Works | CI | — |
| Alacritty | ⚠️ Limited | CI + manual preview | External PDF viewer required |
| GNOME Terminal | ⚠️ Limited | CI | External PDF viewer required |
| iTerm2 | ❓ Needs testing | Pending | — |
| Apple Terminal | ⚠️ Limited | Manual preview | External PDF viewer required |
| Windows Terminal (WSL) | ❓ Needs testing | Pending | — |

CI runs launch, editing, compilation, clipboard, and resize checks on Linux x86-64 (X11).
Manual preview checks cover macOS. Full checks on macOS, WSL, and Linux Wayland are pending.

See [test results and screenshots](https://github.com/hexaquarks/texo/actions/workflows/native-terminals.yml).

App logic and PDF compilation are also tested on Intel and Apple Silicon Macs.
