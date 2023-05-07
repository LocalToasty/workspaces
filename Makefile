.POSIX:

BIN = target/release/workspaces

$(BIN): src/main.rs src/cli.rs src/config.rs src/zfs.rs
	cargo build --release

install: $(BIN)
	cp $(BIN) /usr/local/bin/
	chmod u+s /usr/local/bin/workspaces
	mkdir -p /usr/local/share/workspaces
	cp clean-workspaces.service /etc/systemd/system/
	cp clean-workspaces.timer /etc/systemd/system/
