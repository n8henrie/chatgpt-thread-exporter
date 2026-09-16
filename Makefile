.PHONY: build stage package test lint format ci release-check clean

build: stage package

stage:
	wasm-pack build --target web --out-dir target/wasm-pack --out-name wasm --no-pack --mode no-install -- --locked
	mv target/wasm-pack/wasm.js target/wasm-pack/wasm.d.ts extension
	wasm-opt --enable-bulk-memory-opt -Oz target/wasm-pack/wasm_bg.wasm -o extension/wasm_bg.wasm
	tsc

package:
	mkdir -p dist
	rm -f dist/chatgpt-thread-exporter-firefox.xpi
	cd extension && zip -X "$(CURDIR)/dist/chatgpt-thread-exporter-firefox.xpi" manifest.json popup.html popup.css background.js content.js core.js popup.js wasm.js wasm_bg.wasm icons/icon.svg

test: stage
	cargo test --locked
	node tests/extension.mjs extension

lint: stage
	cargo fmt --check
	cargo clippy --locked --all-targets --all-features -- -D warnings -W clippy::pedantic
	nixfmt --check archive.nix flake.nix package.nix
	actionlint .github/workflows/ci.yml
	web-ext lint --source-dir extension --ignore-files "*.ts" "*.d.ts" --self-hosted --warnings-as-errors
	node tests/audit.mjs . extension

format:
	cargo fmt
	nixfmt archive.nix flake.nix package.nix

ci: build test lint

release-check:
	test "$(RELEASE_TAG)" = "v$$(node -p 'require("./extension/manifest.json").version')"

clean:
	rm -rf dist target
	rm -f extension/background.js extension/content.js extension/core.js extension/popup.js extension/wasm.js extension/wasm.d.ts extension/wasm_bg.wasm
