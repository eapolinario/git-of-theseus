{
  description = "git-of-theseus: Plot stats on Git repositories";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        pythonEnv = pkgs.python3.withPackages (ps: with ps; [
          gitpython
          matplotlib
          numpy
          pygments
          ps."python-dateutil"
          scipy
          tqdm
          wcmatch
          # build / packaging
          hatchling
          pip
        ]);

        # Native deps required to build the Rust crates (git2 -> libgit2,
        # which links against openssl + zlib; libiconv is needed on darwin).
        rustNativeBuildInputs = [ pkgs.pkg-config ];
        rustBuildInputs = [
          pkgs.openssl
          pkgs.zlib
          # Required by `plotters` (font-kit -> fontconfig + freetype) for
          # rendering text in PNG/SVG plots.
          pkgs.fontconfig
          pkgs.freetype
        ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
          pkgs.libiconv
          pkgs.darwin.apple_sdk.frameworks.Security
          pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
          pkgs.darwin.apple_sdk.frameworks.CoreFoundation
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          name = "git-of-theseus";

          nativeBuildInputs = rustNativeBuildInputs;
          buildInputs = rustBuildInputs;

          packages = [
            pythonEnv
            pkgs.uv
            pkgs.just
            pkgs.git
            pkgs.stdenv.cc.cc.lib

            # Rust toolchain
            pkgs.rustc
            pkgs.cargo
            pkgs.clippy
            pkgs.rustfmt
            pkgs.rust-analyzer
          ];

          # Help rust-analyzer find the standard library sources.
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";

          shellHook = ''
            echo "git-of-theseus dev shell"
            echo "  python: $(python3 --version)"
            echo "  rustc:  $(rustc --version)"
            echo "  cargo:  $(cargo --version)"
            echo "Run 'just' to see available commands."
            export LD_LIBRARY_PATH=${pkgs.stdenv.cc.cc.lib}/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
          '';
        };

        # Python package
        packages.default = pkgs.python3Packages.buildPythonPackage {
          pname = "git-of-theseus";
          version = "0.3.4";
          src = ./.;
          pyproject = true;

          build-system = with pkgs.python3Packages; [ hatchling ];

          propagatedBuildInputs = with pkgs.python3Packages; [
            gitpython
            matplotlib
            numpy
            pygments
            pkgs.python3Packages."python-dateutil"
            scipy
            tqdm
            wcmatch
          ];
        };

        # Rust CLI: `nix build .#got-cli`
        packages.got-cli = pkgs.rustPlatform.buildRustPackage {
          pname = "got-cli";
          version = "0.4.0-alpha.1";
          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          nativeBuildInputs = rustNativeBuildInputs;
          buildInputs = rustBuildInputs;

          # The end-to-end test in got-core shells out to `git`.
          nativeCheckInputs = [ pkgs.git ];

          # Build only the CLI binary from the workspace.
          cargoBuildFlags = [ "-p" "got-cli" ];
          cargoTestFlags = [ "--workspace" ];
        };

        # `nix run .#analyze-rs` -> runs the Rust analyzer
        apps.analyze-rs = {
          type = "app";
          program = "${self.packages.${system}.got-cli}/bin/git-of-theseus-analyze-rs";
        };
      }
    );
}
