{
  binaryen,
  lib,
  nodejs,
  rustPlatform,
  typescript,
  wasm-bindgen-cli,
  wasm-pack,
  lld,
}:

let
  cargoToml = lib.importTOML ./Cargo.toml;
in
rustPlatform.buildRustPackage {
  pname = cargoToml.package.name;
  version = cargoToml.package.version;

  src = lib.cleanSource ./.;

  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = [
    binaryen
    lld
    nodejs
    typescript
    # wasm-pack delegates binding generation to this exact-version CLI.
    wasm-bindgen-cli
    wasm-pack
  ];

  # wasm-pack initializes a cache even when tool installation is disabled.
  preBuild = ''
    export WASM_PACK_CACHE="$TMPDIR/wasm-pack"
  '';

  buildPhase = ''
    runHook preBuild
    make stage
    runHook postBuild
  '';

  checkPhase = ''
    runHook preCheck
    make test
    runHook postCheck
  '';

  installPhase = ''
    runHook preInstall
    mkdir "$out"
    cp -r extension/. "$out"
    runHook postInstall
  '';

  meta = {
    inherit (cargoToml.package) description;
    license = lib.licenses.mit;
  };
}
