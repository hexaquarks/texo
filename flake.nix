{
  description = "Texo native terminal smoke tests";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, crane }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
      craneLib = crane.mkLib pkgs;
      texo = craneLib.buildPackage {
        src = pkgs.lib.cleanSourceWith {
          src = self;
          filter = path: type:
            craneLib.filterCargoSources path type
            || builtins.baseNameOf path == "kitty_diacritics.txt";
        };
        strictDeps = true;
        nativeBuildInputs = [ pkgs.pkg-config ];
        buildInputs = [ pkgs.dbus ];
        # Core tests run in the ordinary platform workflow; this build feeds the GUI tests.
        doCheck = false;
      };
    in {
      packages.${system} = {
        inherit texo;
        default = texo;
        prepare-tex-cache = pkgs.writeShellApplication {
          name = "prepare-tex-cache";
          runtimeInputs = [ pkgs.tectonic-unwrapped pkgs.coreutils ];
          text = ''
            if [ "$#" -ne 1 ]; then
              echo "Usage: prepare-tex-cache CACHE_DIRECTORY" >&2
              exit 2
            fi

            mkdir -p "$1"
            export TECTONIC_CACHE_DIR
            TECTONIC_CACHE_DIR="$(realpath "$1")"
            build_directory="$(mktemp -d)"
            trap 'rm -rf "$build_directory"' EXIT

            tectonic --outdir "$build_directory" ${./tests/fixtures/resume.tex}
          '';
        };
      };

      # The cache is seeded with network access, then imported read-only into the offline VM.
      lib.mkTerminalTest = { system, terminal, cache }:
        assert system == "x86_64-linux";
        import ./tests/native/terminal.nix {
          inherit pkgs texo terminal cache;
          fixture = ./tests/fixtures/resume.tex;
        };
    };
}
