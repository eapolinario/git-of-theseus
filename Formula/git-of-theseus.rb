# This formula is automatically updated (version, download URLs, and
# checksums) by .github/workflows/release.yml whenever a new `vX.Y.Z` tag is
# pushed. See "Homebrew" in README.md for installation and upgrade
# instructions, and docs on how updates are published.
class GitOfTheseus < Formula
  desc "Analyze the evolution of a Git repository's code over time"
  homepage "https://github.com/eapolinario/git-of-theseus"
  version "0.0.0"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/eapolinario/git-of-theseus/releases/download/v#{version}/git-of-theseus-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end

    on_intel do
      url "https://github.com/eapolinario/git-of-theseus/releases/download/v#{version}/git-of-theseus-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  def install
    bin.install "git-of-theseus-analyze"
    bin.install "git-of-theseus-line-plot"
    bin.install "git-of-theseus-stack-plot"
    bin.install "git-of-theseus-survival-plot"
  end

  test do
    %w[analyze line-plot stack-plot survival-plot].each do |subcommand|
      assert_match "Usage", shell_output("#{bin}/git-of-theseus-#{subcommand} --help")
    end
  end
end
