# Makefile for NetWatch

APP_NAME := netwatch
TARGET_DIR := target

# Optimization flags for small binary size
# -C embed-bitcode=yes: Required for LTO
# -C strip=symbols: Strip debug info and symbols (requires Rust 1.59+)
# -C opt-level=z: Optimize for size
# -C lto=true: Link Time Optimization
# -C panic=abort: Remove unwinding code
# -C codegen-units=1: Maximize optimization
RUSTFLAGS_SIZE := -C embed-bitcode=yes -C strip=symbols -C opt-level=z -C lto=true -C panic=abort -C codegen-units=1

# Default target
.PHONY: all
all: x86_64 aarch64

# x86_64-unknown-linux-musl (Static binary for x86)
# Uses x86_64-linux-gcc as linker and optimized for size
.PHONY: x86_64
x86_64:
	@echo "Building for x86_64 musl..."
	CC_x86_64_unknown_linux_musl=x86_64-linux-gcc \
	CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=x86_64-linux-gcc \
	RUSTFLAGS="$(RUSTFLAGS_SIZE)" \
	cargo build --release --target x86_64-unknown-linux-musl

# aarch64-unknown-linux-musl
# Uses aarch64-linux-gcc as linker and optimized for size
.PHONY: aarch64
aarch64:
	@echo "Building for aarch64 musl..."
	CC_aarch64_unknown_linux_musl=aarch64-linux-gcc \
	CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-gcc \
	RUSTFLAGS="$(RUSTFLAGS_SIZE)" \
	cargo build --release --target aarch64-unknown-linux-musl

# Clean
.PHONY: clean
clean:
	cargo clean
