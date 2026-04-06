class Radisk < Formula
  desc "Terminal-based radial disk usage visualizer inspired by KDE FileLight"
  homepage "https://github.com/mimobn/radisk"
  url "https://github.com/mimobn/radisk/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "3fe978c712f32eb74e7d5f70c1b98c4369e9e0bc55b07e90b21d87bb47e811c2"
  license "GPL-3.0-or-later"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "radisk", shell_output("#{bin}/radisk --help")
  end
end
