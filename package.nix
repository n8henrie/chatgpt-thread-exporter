{
  binaryen,
  lib,
  makeRustPlatform,
  nodejs,
  rust-bin,
  typescript,
  wasm-bindgen-cli,
  wasm-pack,
  zip,
}:

let
  cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
  toolchain = rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
  rustPlatform = makeRustPlatform {
    cargo = toolchain;
    rustc = toolchain;
  };
in
rustPlatform.buildRustPackage {
  pname = cargoToml.package.name;
  version = cargoToml.package.version;
  outputs = [
    "out"
    "unpacked"
  ];

  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./Cargo.toml
      ./Cargo.lock
      ./Makefile
      ./extension
      ./rust-toolchain.toml
      ./src
      ./tests/export_plan.rs
      ./tests/extension.mjs
      ./tests/fixtures
      ./tsconfig.json
    ];
  };

  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = [
    binaryen
    nodejs
    typescript
    # wasm-pack selects the wasm-bindgen CLI version from Cargo.lock.
    wasm-bindgen-cli
    wasm-pack
    zip
  ];

  # wasm-pack initializes its cache even when installation is disabled.
  preBuild = ''
    export WASM_PACK_CACHE="$TMPDIR/wasm-pack"
  '';

  buildPhase = ''
    runHook preBuild
    make build
    runHook postBuild
  '';

  checkPhase = ''
    runHook preCheck
    make test
    runHook postCheck
  '';

  installPhase = ''
    runHook preInstall
    mkdir "$out" "$unpacked"
    cp dist/chatgpt-thread-exporter-firefox.xpi "$out/"
    cp -r extension/. "$unpacked"
    runHook postInstall
  '';

  meta = {
    inherit (cargoToml.package) description;
    license = lib.licenses.mit;
  };
}
