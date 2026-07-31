.PHONY: build-linux

# Compila release Linux x86_64 dentro de container Docker (sem precisar de máquina Linux física).
# --platform=linux/amd64 força arch x86_64 mesmo em Mac Apple Silicon (via QEMU do Docker Desktop) —
# hardware alvo (VLT) é x86_64, não arm64.
build-linux:
	docker build --platform=linux/amd64 -t pachinko-launcher-builder -f Dockerfile.build .
	docker run --rm --platform=linux/amd64 \
		-v "$(CURDIR)":/app \
		-v pachinko-launcher-cargo-amd64:/usr/local/cargo/registry \
		-w /app \
		pachinko-launcher-builder \
		cargo build --release
	@echo "Binário em target/release/pachinko_vlt_launcher"
