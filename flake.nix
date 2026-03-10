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
          python-dateutil
          scipy
          tqdm
          wcmatch
          # build / packaging
          hatchling
          pip
        ]);
      in
      {
        devShells.default = pkgs.mkShell {
          name = "git-of-theseus";

          packages = [
            pythonEnv
            pkgs.uv
            pkgs.just
            pkgs.git
          ];

          shellHook = ''
            echo "git-of-theseus dev shell"
            echo "Run 'just' to see available commands."
          '';
        };

        # Expose the package itself so 'nix build' works too
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
            python-dateutil
            scipy
            tqdm
            wcmatch
          ];
        };
      }
    );
}
