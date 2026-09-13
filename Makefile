.PHONY: build stage package test lint format ci release-check clean

build: package

stage:
	wasm-pack build --target web --out-dir extension --no-pack --mode no-install -- --locked
	rm -f extension/.gitignore
	tsc

package: stage
	mkdir -p dist
	rm -f dist/chatgpt-thread-exporter-firefox.xpi
	cd extension && zip -r -X ../dist/chatgpt-thread-exporter-firefox.xpi manifest.json popup.html popup.css background.js content.js core.js popup.js chatgpt_thread_exporter.js chatgpt_thread_exporter_bg.wasm icons

test: stage
	cargo test --locked
	node tests/extension.mjs extension

lint: stage
	cargo fmt --check
	cargo clippy --locked --all-targets --all-features -- -D warnings -W clippy::pedantic
	nixfmt --check flake.nix package.nix
	actionlint .github/workflows/ci.yml
	node tests/audit.mjs . extension

format:
	cargo fmt
	nixfmt flake.nix package.nix

ci: build test lint

release-check:
	test "$(RELEASE_TAG)" = "v$$(node -p 'require("./extension/manifest.json").version')"

clean:
	rm -rf dist target
	rm -f extension/.gitignore extension/background.js extension/chatgpt_thread_exporter.d.ts extension/chatgpt_thread_exporter.js extension/chatgpt_thread_exporter_bg.wasm extension/chatgpt_thread_exporter_bg.wasm.d.ts extension/content.js extension/core.js extension/popup.js
