{ pkgs, ... }:
let
  hash = "sha256-hL0VGY3zb10+TFu4JuLBoqHWPAOQbov9Kw79zppf3bE=";
  bundle = pkgs.fetchurl {
    url = "https://github.com/ibisjp/ibis-office/releases/download/v0.8.9/ibisOffice-0.8.9-Linux-x86_64.flatpak";
    inherit hash;
  };
in
{
  services.flatpak.enable = true;
  services.flatpak.packages = [
    {
      appId = "jp.ne.ibis.ibisOffice";
      bundle = "${bundle}";
      sha256 = hash;
    }
  ];
}
