target/release/workspace: src/main.rs
	cargo build --release

install: target/release/workspace
	install --mode 4755 target/release/workspace /usr/local/bin/workspace