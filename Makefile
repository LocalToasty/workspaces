.POSIX:

BIN = target/release/workspaces

$(BIN): src/main.rs
	cargo build --release

install: $(BIN)
	cp $(BIN) /usr/local/bin/
	chmod 4755 $(BIN)
	mkdir -p /usr/local/share/workspaces
	cp clean-workspaces.service /etc/systemd/system/
	cp clean-workspaces.timer /etc/systemd/system/