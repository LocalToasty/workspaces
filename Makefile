target/release/workspace: src/main.rs
	cargo build --release

install: target/release/workspace
	install --mode 4755 -T target/release/workspace /usr/local/bin/workspace
	mkdir -p /usr/local/share/workspaces
