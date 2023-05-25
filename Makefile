.POSIX:

BIN = target/release/workspaces

$(BIN): src/main.rs src/cli.rs src/config.rs src/zfs.rs
	cargo build --release

install: $(BIN)
	# install binary
	install -D -m 4755 $(BIN) /usr/local/bin/workspaces
	test -e /usr/bin/workspaces || ln -s /usr/local/bin/workspaces /usr/bin/workspaces
	# copy config
	mkdir -p /etc/workspaces
	cp workspaces.toml /etc/workspaces/workspaces.example.toml
	test -e /etc/workspaces/workspaces.toml || cp workspaces.toml /etc/workspaces/
	# make database dir
	mkdir -p /usr/local/lib/workspaces
	#TODO this will be removed for version 0.4.0
	# move already existing database to new location if necessary
	test -e /usr/local/share/workspaces/workspaces.db && mv /usr/local/share/workspaces/workspaces.db /usr/local/lib/workspaces/workspaces.db
	# install systemd service / timer
	cp clean-workspaces.service /etc/systemd/system/
	cp clean-workspaces.timer /etc/systemd/system/
	systemctl daemon-reload
