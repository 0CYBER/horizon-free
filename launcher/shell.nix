# Build environment for the launcher on NixOS: `nix-shell` then `cargo build`.
{ pkgs ? import <nixpkgs> {} }:
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [ pkg-config clang mold ];
  buildInputs = with pkgs; [
    libxkbcommon wayland vulkan-loader fontconfig freetype alsa-lib openssl
    libx11 libxcb libxcursor libxrandr libxi
  ];
  LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
  LD_LIBRARY_PATH = with pkgs; lib.makeLibraryPath [ libxkbcommon wayland vulkan-loader fontconfig freetype libx11 libxcb libxcursor libxrandr libxi ];
}
