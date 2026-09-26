# Releasing through WinGet

The release workflow builds the Windows x64 and arm64 archives, then
generates a single WinGet manifest covering both architectures. The
manifest is validated with `winget validate` on a Windows runner and
attached to the GitHub Release as three YAML assets:

- `Eapolinario.GitOfTheseus.yaml`
- `Eapolinario.GitOfTheseus.installer.yaml`
- `Eapolinario.GitOfTheseus.locale.en-US.yaml`

The installer manifest declares one `Installers` entry per architecture
(`x64` and `arm64`), each pointing at its tagged GitHub Release ZIP and
using its SHA-256. It declares every executable in that archive as a
portable command alias, so all four commands are added to `PATH` on
either architecture.

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
4. After that pull request merges, verify an upgrade on Windows x64. Install
   the previous published version first, then upgrade to the new release:

   ```powershell
   winget install --id Eapolinario.GitOfTheseus --version <previous-version> --exact
   git-of-theseus-analyze --help
   git-of-theseus-line-plot --help
   git-of-theseus-stack-plot --help
   git-of-theseus-survival-plot --help

   winget upgrade --id Eapolinario.GitOfTheseus --exact
   ```

   Run the four `--help` commands again after the upgrade.
