{
  description = "Rust/WASM Firefox extension for exporting ChatGPT threads";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, rust-overlay, ... }:
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
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
        extension = pkgs.callPackage ./package.nix { };
      in
      {
        packages = {
          default = extension;
          debug = extension.unpacked;
        };

        checks.default = extension;

        devShells.default = pkgs.mkShell {
          inputsFrom = [ extension ];
          packages = [
            pkgs.actionlint
            pkgs.nixfmt
          ];
        };

        formatter = pkgs.nixfmt;
      }
    );
}
