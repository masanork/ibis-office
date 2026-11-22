# ibisOffice on NixOS

These examples target the upstream [ibisOffice v0.8.9 release](https://github.com/ibisjp/ibis-office/releases/tag/v0.8.9) discussed in [Issue #269](https://github.com/ibisjp/ibis-office/issues/269).

## Flatpak on x86_64 NixOS

Import [the nix-flatpak v0.8.0 NixOS module](https://github.com/gmodena/nix-flatpak/tree/v0.8.0) and the local `flatpak.nix` into your NixOS configuration. For example, with the nix-flatpak flake input pinned to `github:gmodena/nix-flatpak/v0.8.0`:

```nix
modules = [
  nix-flatpak.nixosModules.nix-flatpak
  ./flatpak.nix
];
```

Apply the configuration with `sudo nixos-rebuild switch`. The bundle is downloaded from the official release and checked against its published SHA-256. The module installs it system-wide with the Freedesktop 24.08 runtime from Flathub. In a desktop session, launch it with:

```sh
flatpak run jp.ne.ibis.ibisOffice
```

On NixOS 26.05 x86_64 with Xfce/X11 in QEMU/KVM, the declarative install survived a guest reboot. The application opened a synthetic CSV and DOCX. QEMU's display emitted a DRI3 warning while rendering the tested files.

## Build the desktop app from source

```sh
nix-build packaging/nix \
  -I nixpkgs=https://github.com/NixOS/nixpkgs/archive/2ce3c67a3687fca89a7342d40bfa28aff0b6a114.tar.gz
./result/bin/ibisOffice /path/to/document.docx
```

ibisOffice v0.8.9 uses `Option::map_or_default`, which requires Rust 1.98 or newer. The NixOS 26.05 channel's Rust 1.95 failed to compile it. The pinned Nixpkgs revision provides Rust 1.99.1 and can be used from the 26.05 guest without switching the running system.

The definition in `package.nix` uses the upstream release's pinned Git submodules and Cargo lockfile. Nix verifies the source and dependency hashes before the sandboxed build. The package installs the desktop entry, application icons, notices, and ECMA art border assets.

The v0.8.9 source tag omits the fixture `2026-11-14-179-rest-many-patterns-result.tsv` referenced by the DOCX scenario tests, so the package disables the application Cargo test hook. The application can be smoke-tested by opening a synthetic CSV or DOCX fixture after installation.

The pinned revision built and installed successfully with sandboxing enabled on the x86_64 NixOS 26.05 guest. The installed executable opened a synthetic CSV and DOCX under X11.
