class Radisk < Formula
  desc "Terminal-based radial disk usage visualizer inspired by KDE FileLight"
  homepage "https://github.com/mimobn/radisk"
  url "https://github.com/mimobn/radisk/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "" # Fill after release
  license "GPL-3.0-or-later"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "radisk", shell_output("#{bin}/radisk --help")
  end
end
