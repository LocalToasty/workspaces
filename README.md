# Workspaces

Workspaces are a way to manage ephemeral data. They allow users to create
folders that are automatically deleted if they haven't been used for a certain
period of time. This is a convenient method for handling temporary files and
preventing clutter on file systems.

Workspaces uses ZFS to efficiently manage the creation, extension, and deletion
of workspaces.

## Installation

Before installing Workspaces, you must have Rust installed. To install and build
Workspaces, run the following command:
```console
$ make && sudo make install
```
You must manually modify the `/etc/workspaces/workspaces.toml` file, and you
must have already set up a ZFS zpool.

To activate automatic deletion of old workspaces, enable the corresponding
systemd timer:
```console
$ sudo systemctl enable --now clean-workspaces.timer
```

## User Tutorial

This tutorial will walk you through the process of using Workspaces, including
creating a workspace, extending its expiry date, and manually expiring it.

### Creating a Workspace

Use the `workspaces filesystems` command to display the available filesystems
and their associated details, including the amount of free space, the maximum
time you can initially create a workspace for, and the number of days a
read-only copy of your workspace will be kept after it expires before it is
finally deleted:
```console
$ workspaces filesystems
FILESYSTEM     	FREE   	DURATION	RETENTION
bulk           	  9.40T	     90d	      30d
ssd            	   321G	     30d	       7d
```

To create a workspace named `testws` on the `bulk` filesystem with a ten-day
expiry date, enter:
```console
$ workspaces create -f bulk -d 10 testws
Created workspace at /mnt/bulk/mvantreeck/testws
```

Use the `workspaces list` command to view all available workspaces:
```console
$ workspaces list
NAME                   	USER           	FILESYSTEM     	EXPIRY DATE   	SIZE  	MOUNTPOINT
testws                 	mvantreeck     	bulk           	expires in  9d	   96K	/mnt/bulk/mvantreeck/testws
```

You may now use your workspace like any other folder:
```console
$ echo "Hello workspaces" > /mnt/bulk/mvantreeck/testws/testfile
$ workspaces list
NAME                   	USER           	FILESYSTEM     	EXPIRY DATE   	SIZE  	MOUNTPOINT
testws                 	mvantreeck     	bulk           	expires in  9d	  104K	/mnt/bulk/mvantreeck/testws
```

### Extending a Workspace

If you need to extend the expiry date of your workspace, use the `extend`
command:
```console
$ workspaces list
NAME                   	USER           	FILESYSTEM     	EXPIRY DATE   	SIZE  	MOUNTPOINT
testws                 	mvantreeck     	bulk           	expires in  2d	  104K	/mnt/bulk/mvantreeck/testws
$ workspaces extend -f bulk -d 16 testws
$ workspaces list
NAME                   	USER           	FILESYSTEM     	EXPIRY DATE   	SIZE  	MOUNTPOINT
testws                 	mvantreeck     	bulk           	expires in 15d	  104K	/mnt/bulk/mvantreeck/testws
```

If you fail to extend your workspace in time, it will expire and become
read-only:
```console
$ workspaces list
NAME                   	USER           	FILESYSTEM     	EXPIRY DATE   	SIZE  	MOUNTPOINT
testws                 	mvantreeck     	bulk           	deleted in 23d	  104K	/mnt/bulk/mvantreeck/testws
$ touch /mnt/bulk/mvantreeck/testws/testfile
touch: cannot touch '/mnt/bulk/mvantreeck/testws/testfile': Read-only file system
```

However, you can make it writable again by extending it once more:
```console
$ workspaces extend -f bulk -d 3 testws
$ workspaces list
NAME                   	USER           	FILESYSTEM     	EXPIRY DATE   	SIZE  	MOUNTPOINT
test1                  	mvantreeck     	bulk           	expires in  2d	  104K	/mnt/bulk/mvantreeck/test1
$ touch /mnt/bulk/mvantreeck/test1/testfile	# completes successfully
```

If you don't extend the workspace in time, it will eventually be deleted.

### Manually Expiring a Workspace

To manually expire a workspace that is no longer needed, you can use the
`expire` command. After running this command, the workspace will will become
read-only and marked for eventualy deletion:
```console
$ workspaces expire -f bulk testws
$ workspaces list
NAME                   	USER           	FILESYSTEM     	EXPIRY DATE   	SIZE  	MOUNTPOINT
testws                 	mvantreeck     	bulk           	deleted in 29d	  104K	/mnt/bulk/mvantreeck/testws
$ touch /mnt/bulk/mvantreeck/testws/testfile
touch: cannot touch '/mnt/bulk/mvantreeck/testws/testfile': Read-only file system
```

If you change your mind and decide you need the workspace again before its final
deletion date, you can extend its expiry date using the `extend` command.

### Manually Running the Garbage Collector

Usually, your system administrator will have set up the garbage collector to
automatically clean up workspaces that are flagged for deletion once in a while.
However, you can also manually run the garbage collector using the clean
command:
```console
$ workspaces list
NAME                   	USER           	FILESYSTEM     	EXPIRY DATE   	SIZE  	MOUNTPOINT
testws                 	mvantreeck     	bulk           	deleted   soon	  104K	/mnt/bulk/mvantreeck/testws
$ workspaces clean
NAME                   	USER           	FILESYSTEM     	EXPIRY DATE   	SIZE  	MOUNTPOINT
[ no workspaces ]
```
