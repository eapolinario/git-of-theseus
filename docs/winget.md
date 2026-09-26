# Releasing through WinGet

The release workflow creates a WinGet manifest for the Windows x64 archive
after building it. The manifest is validated with `winget validate` on the
Windows runner and attached to the GitHub Release as three YAML assets:

- `Eapolinario.GitOfTheseus.yaml`
- `Eapolinario.GitOfTheseus.installer.yaml`
- `Eapolinario.GitOfTheseus.locale.en-US.yaml`

The installer manifest points at the tagged GitHub Release ZIP and uses its
SHA-256. It declares every executable in that archive as a portable command
alias, so all four commands are added to `PATH`.

## Submitting a tagged release

1. Wait for the GitHub Release workflow for the `v<version>` tag to finish.
   Download the three generated YAML assets and place them at
   `manifests/e/Eapolinario/GitOfTheseus/<version>/` in a checkout of
   [`microsoft/winget-pkgs`](https://github.com/microsoft/winget-pkgs).
2. On Windows, validate the directory with:

   ```powershell
   winget validate --manifest manifests/e/Eapolinario/GitOfTheseus/<version>
   ```

3. Submit the directory in a pull request to `microsoft/winget-pkgs`. The
   submission must retain the generated URL, version, SHA-256, and all four
   `NestedInstallerFiles` aliases; the community repository's validation
   downloads the release asset and verifies those values.
4. After that pull request merges, verify a fresh installation and an upgrade
   on Windows x64:

   ```powershell
   winget install --id Eapolinario.GitOfTheseus --exact
   git-of-theseus-analyze --help
   git-of-theseus-line-plot --help
   git-of-theseus-stack-plot --help
   git-of-theseus-survival-plot --help

   winget upgrade --id Eapolinario.GitOfTheseus --exact
   ```

   Run the four `--help` commands again after the upgrade. If the package is
   already installed, use `winget upgrade` directly instead of `install`.
