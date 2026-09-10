# Homebrew formula template for tlog.
#
# Publish with:
#   brew create --tap <user>/<tap> https://github.com/ryans233/tlog
# then replace this file and fill in the per-arch SHA256 values from the
# GitHub Release `SHA256SUMS` files.
class Tlog < Formula
  desc "Android Logcat TUI viewer with inline terminal scrollback"
  homepage "https://github.com/ryans233/tlog"
  url "https://github.com/ryans233/tlog/releases/download/v1.4.2/tlog-aarch64-apple-darwin.tar.gz"
  version "1.4.2"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/ryans233/tlog/releases/download/v1.4.2/tlog-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_APPLE_SILICON_SHA256"
    else
      url "https://github.com/ryans233/tlog/releases/download/v1.4.2/tlog-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_APPLE_INTEL_SHA256"
    end
  end

  on_linux do
    url "https://github.com/ryans233/tlog/releases/download/v1.4.2/tlog-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "REPLACE_WITH_LINUX_X86_64_SHA256"
  end

  def install
    bin.install "tlog"
  end

  test do
    assert_match "tlog #{version}", shell_output("#{bin}/tlog --version")
  end
end
