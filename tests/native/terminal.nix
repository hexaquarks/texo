{ pkgs, texo, terminal, cache, fixture }:

let
  # Adapted from ratatui-image's screenshot-tests.nix (MIT) and nixpkgs' terminal-emulators test.
  # https://github.com/ratatui/ratatui-image/blob/master/screenshot-tests.nix
  terminals = {
    kitty = {
      package = pkgs.kitty;
      command = "kitty --config NONE --override font_size=16";
    };
    ghostty = {
      package = pkgs.ghostty;
      command = "ghostty --gtk-single-instance=false --font-size=16 -e";
    };
    wezterm = {
      package = pkgs.wezterm;
      command = "wezterm --skip-config --config font_size=16 start --always-new-process --";
    };
    alacritty = {
      package = pkgs.alacritty;
      command = "env WINIT_X11_SCALE_FACTOR=1 alacritty --option font.size=16 -e";
    };
    gnome-terminal = {
      package = pkgs.gnome-terminal;
      command = "gnome-terminal --";
    };
  };
  selected = terminals.${terminal};
  launch = pkgs.writeShellScript "texo-native-test" ''
    export TECTONIC_CACHE_DIR=${cache}
    export NO_COLOR=""
    ${texo}/bin/texo /home/alice/workspace/resume.tex 2>/home/alice/texo.log
    echo "$?" > /home/alice/texo-exit-status
  '';
in
pkgs.testers.nixosTest {
  name = "texo-native-${terminal}";
  enableOCR = true;

  nodes.machine = { ... }: {
    imports = [
      "${pkgs.path}/nixos/tests/common/x11.nix"
      "${pkgs.path}/nixos/tests/common/user-account.nix"
    ];
    test-support.displayManager.auto.user = "alice";
    virtualisation.memorySize = 4096;
    virtualisation.cores = 2;
    virtualisation.resolution = { x = 1920; y = 1080; };
    fonts.packages = [ pkgs.dejavu_fonts ];
    environment.systemPackages = [
      selected.package
      pkgs.bubblewrap
      pkgs.tectonic-unwrapped
      pkgs.poppler-utils
      pkgs.xdotool
      pkgs.xclip
    ];
  };

  testScript = ''
    terminal_command = ${builtins.toJSON "${selected.command} ${launch}"}
    fixture = ${builtins.toJSON "${fixture}"}
    terminal_name = ${builtins.toJSON terminal}
    terminal_version = ${builtins.toJSON selected.package.version}
    ${builtins.readFile ./smoke.py}
  '';
}
