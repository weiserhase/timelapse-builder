NAME        := timelapse-builder
BIN         := timelapse
DIST        := dist
WIN_TARGET  := x86_64-pc-windows-gnu
ARCH        := $(shell uname -m)
ARGS        ?=

.DEFAULT_GOAL := help

.PHONY: build
build: ## Debug build
	cargo build

.PHONY: release
release: ## Optimized release build
	cargo build --release

.PHONY: test
test: ## Run tests
	cargo test

.PHONY: fmt
fmt: ## Format the code
	cargo fmt

.PHONY: clippy
clippy: ## Lint with clippy
	cargo clippy --all-targets -- -D warnings

.PHONY: run
run: ## Run: make run ARGS="./photos -o out.mp4"
	cargo run --release -- $(ARGS)

.PHONY: bundle
bundle: release ## Linux self-contained folder + tar.gz
	@rm -rf "$(DIST)/$(NAME)-linux-$(ARCH)"
	@mkdir -p "$(DIST)/$(NAME)-linux-$(ARCH)"
	cp target/release/$(BIN) "$(DIST)/$(NAME)-linux-$(ARCH)/"
	scripts/fetch-ffmpeg.sh linux "$(DIST)/.ff-linux" >/dev/null
	cp "$(DIST)/.ff-linux/ffmpeg" "$(DIST)/$(NAME)-linux-$(ARCH)/"
	cp README.md "$(DIST)/$(NAME)-linux-$(ARCH)/" 2>/dev/null || true
	tar -czf "$(DIST)/$(NAME)-linux-$(ARCH).tar.gz" -C "$(DIST)" "$(NAME)-linux-$(ARCH)"
	@echo ">> $(DIST)/$(NAME)-linux-$(ARCH).tar.gz"

.PHONY: windows
windows: ## Cross-compile timelapse.exe (needs: rustup target add + mingw-w64)
	@command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1 || { \
	  echo "!! mingw-w64 linker not found. Install it (Debian: sudo apt install mingw-w64),"; \
	  echo "   or just push a tag and let GitHub Actions build the Windows binary."; exit 1; }
	rustup target add $(WIN_TARGET)
	cargo build --release --target $(WIN_TARGET)
	@echo ">> target/$(WIN_TARGET)/release/$(BIN).exe"

.PHONY: win-bundle
win-bundle: windows ## Windows self-contained folder + zip
	@rm -rf "$(DIST)/$(NAME)-windows-x86_64"
	@mkdir -p "$(DIST)/$(NAME)-windows-x86_64"
	cp target/$(WIN_TARGET)/release/$(BIN).exe "$(DIST)/$(NAME)-windows-x86_64/"
	scripts/fetch-ffmpeg.sh windows "$(DIST)/.ff-win" >/dev/null
	cp "$(DIST)/.ff-win/ffmpeg.exe" "$(DIST)/$(NAME)-windows-x86_64/"
	cp README.md "$(DIST)/$(NAME)-windows-x86_64/" 2>/dev/null || true
	cd "$(DIST)" && zip -qr "$(NAME)-windows-x86_64.zip" "$(NAME)-windows-x86_64"
	@echo ">> $(DIST)/$(NAME)-windows-x86_64.zip"

.PHONY: clean
clean: ## Remove build output and bundles
	cargo clean
	rm -rf "$(DIST)"

.PHONY: help
help: ## Show this help
	@grep -hE '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) \
	  | awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2}'
