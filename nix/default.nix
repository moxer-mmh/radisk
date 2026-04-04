{ lib, rustPlatform, fetchFromGitHub }:

rustPlatform.buildRustPackage rec {
  pname = "radisk";
  version = "0.1.0";

  src = fetchFromGitHub {
    owner = "mimobn";
    repo = "radisk";
    rev = "v${version}";
    hash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
  };

  cargoHash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

  meta = with lib; {
    description = "Terminal-based radial disk usage visualizer inspired by KDE FileLight";
    homepage = "https://github.com/mimobn/radisk";
    license = licenses.gpl3Plus;
    mainProgram = "radisk";
    platforms = platforms.all;
  };
}
