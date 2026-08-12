# historia

Minimal, git-style version control for backing up **one folder**. A CLI, written
in Rust.

historia snapshots the current state of a folder and lets you restore any past
state. Linear, numbered history - that is the whole job. It is not git: no
branches, no merge, no staging area, no remotes, no history rewriting. Those are
collaboration features; historia is a personal backup tool.

## Commands (MVP)

```
historia init [dir]              # create store (.historia/) - defaults to current dir
historia commit -m "msg"         # snapshot the whole folder            (alias: snapshot)
historia log                     # list snapshots: number, time, message
historia status                  # what changed since the last snapshot
historia restore <n>             # restore whole folder to snapshot <n> (exact mirror)
historia restore <n> <path>      # restore a single file from snapshot <n>
historia verify                  # re-hash all blobs, check store integrity
historia help [command]          # list commands, or detail one   (aliases: -h, --help, man, info)
historia --version               # version
```

As of this checkpoint (CP0), only `--version` and `help` are implemented; every
other command reports which checkpoint will implement it. See `CLAUDE.md` for the
full design and the checkpoint plan.

## Install

TBD until CP10 (release packaging) - the eventual install will be: download a
single self-contained binary, put it on PATH.

For development:

```
cargo install --path .
```

## License

MIT
