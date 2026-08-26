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

See `CLAUDE.md` for the full design and the checkpoint plan.

## Install

Download the archive for your platform from the
[Releases page](https://github.com/leotamminen/historia/releases), extract it,
and put the `historia` binary on your `PATH`. Each archive is self-contained -
no Rust toolchain, no installer, no external DLLs on Windows.

| Platform               | Asset                                                |
|------------------------|-------------------------------------------------------|
| Windows x64            | `historia-<version>-x86_64-pc-windows-msvc.zip`       |
| Windows ARM64          | `historia-<version>-aarch64-pc-windows-msvc.zip`      |
| macOS (Intel)          | `historia-<version>-x86_64-apple-darwin.tar.gz`       |
| macOS (Apple Silicon)  | `historia-<version>-aarch64-apple-darwin.tar.gz`      |
| Linux x64              | `historia-<version>-x86_64-unknown-linux-gnu.tar.gz`  |
| Linux ARM64            | `historia-<version>-aarch64-unknown-linux-gnu.tar.gz` |

**Known limitation:** the Linux builds link glibc dynamically, so they run on
most modern distributions but need a reasonably current glibc - they will not
run on very old ones. A fully static `musl` build is planned for a later
checkpoint but isn't available yet.

### From source

For development, or any platform not listed above:

```
cargo install --path .
```

## License

MIT
