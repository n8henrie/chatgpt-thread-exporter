{
  description = "Rust/WASM Firefox extension for exporting ChatGPT threads";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { nixpkgs, ... }:
    let
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-darwin"
        "x86_64-linux"
      ];
      eachSystem =
        with nixpkgs.lib;
        f: foldAttrs mergeAttrs { } (map (s: mapAttrs (_: v: { ${s} = v; }) (f s)) systems);
    in
    eachSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        unpacked = pkgs.callPackage ./package.nix { };
        archive = pkgs.callPackage ./archive.nix { extension = unpacked; };
      in
      {
        packages = {
          default = archive;
          debug = unpacked;
        };

        checks.default = archive;

        devShells.default = pkgs.mkShell {
          inputsFrom = [ unpacked ];
          packages = [
            pkgs.actionlint
            pkgs.clippy
            pkgs.nixfmt
            pkgs.rustfmt
            pkgs.web-ext
            pkgs.zip
          ];
        };

        formatter = pkgs.nixfmt;
      }
    );
}
