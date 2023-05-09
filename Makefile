.POSIX:

BIN = target/release/workspaces

$(BIN): src/main.rs src/cli.rs src/config.rs src/zfs.rs
	cargo build --release

install: $(BIN)
	cp $(BIN) /usr/local/bin/workspaces
	chmod u+s /usr/local/bin/workspaces
	ln -s /usr/local/bin/workspaces /usr/local/sbin/workspaces
	mkdir -p /usr/local/share/workspaces
	cp workspaces.toml /usr/local/etc/workspaces.example.toml
	test -f /usr/local/etc/workspaces.toml || cp workspaces.toml /usr/local/etc/workspaces.toml
	cp clean-workspaces.service /etc/systemd/system/
	cp clean-workspaces.timer /etc/systemd/system/
