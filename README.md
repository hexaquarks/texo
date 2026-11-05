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

Terminal smoke tests run on Linux x86-64 (X11) in NixOS VMs.

| Terminal | PDF preview | Smoke tests |
| --- | --- | --- |
| Kitty | ✅ Full graphics | ✅ |
| Ghostty | ✅ Full graphics | ✅ |
| WezTerm | ✅ Full graphics | ✅ |
| Alacritty | Limited: block-based fallback | ✅ |
| GNOME Terminal | Limited: block-based fallback | ✅ |

The block-based fallback shows page layout but is unsuitable for reading PDF text.
See [terminal CI](https://github.com/hexaquarks/texo/actions/workflows/native-terminals.yml) for screenshots and results.

Core and compiler tests also run on Intel and Apple Silicon macOS. Actual macOS terminal apps,
WSL, and Wayland are not yet covered by the terminal smoke tests.
