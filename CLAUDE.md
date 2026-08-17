# CLAUDE.md — historia

Minimal, git-style version control for backing up **one folder**. CLI, written in Rust.
This file is the single auto-loaded context for Claude Code. Update it after every checkpoint.

---

## 1. What historia is (and is not)

historia snapshots the current state of a single folder and lets you restore any past
state. Linear, numbered history. That is the whole job.

**It is NOT:** git. No branches, no merge, no staging area, no remotes, no history
rewriting, no pull requests or issues. Those are collaboration features; historia is a
personal backup tool.

Primary use: backing up code folders ("repos") in day-to-day work. Typical size is small
(NIGHTFALL is ~116 MB and growing — no large media). Big, churning binaries are out of scope.

---

## 2. Why these choices (justify before changing)

**Why Rust?**
- Compiles to a *single self-contained binary* — the user installs one file, no runtime,
  no interpreter, no toolchain. For a tool whose job is not losing files, no GC and no
  runtime means fewer moving parts and no interpreter to install on the target machine.
- Memory-safe without a GC: the write path (the dangerous part) won't segfault mid-write.
- The crate ecosystem maps almost 1:1 onto our feature list (see §7): `ignore` is
  ripgrep's own gitignore matcher; `sha2`/`blake3`, `ed25519-dalek`, `age`, `clap`.
- Strong backward-compatibility via editions; `std::fs`/`std::path` are cross-platform.
- Deliberately a new language for this author (avoiding the usual TS/JS/Python).

**Why a git-style CLI instead of just using git?**
- git tracks content, but its model is built for *source collaboration*: staging,
  branches, merges, remotes, and four overlapping and partly *destructive* restore
  commands (`checkout`, `reset`, `restore`, `revert` — `reset --hard` destroys work).
  For "protect one folder, restore to any past state" that model is overkill and its
  restore UX is a footgun.
- Backing up code folders with git would mean nesting repos inside their own `.git`.
  historia ignores nested `.git` by default and stores its own history separately.
- We keep git *syntax* where it is safe, because the author already has git muscle memory
  (see §5), and deviate only where git is itself confusing or dangerous.
- Learning value (systems / security) and full control over an offline, self-contained,
  future-proof tool.

**Why CLI (not a GUI)?**
- Scriptable: an external scheduler (Windows Task Scheduler, cron) can call it (see §6).
- Fast, and the right shape for a tool run at a folder root.

**Why offline-only?** See Rule 2. The tool must run in air-gapped environments; a backup
tool you can't trust without a network connection is not a backup tool.

---

## 3. Non-negotiable rules

These override convenience. Do not violate them without an explicit decision recorded here.

1. **The author writes every commit.** Claude Code never runs `git commit` or `git push`.
   When a feature or checkpoint is complete, Claude Code prints exactly:
   `✔ <checkpoint> done. Suggested commit message: "<text>"`
   Then stops. The author reviews the code, writes the commit himself (verbatim or edited),
   and pushes himself. This is the most important rule in this file.

2. **No network at runtime — ever.** Every command must work fully offline and air-gapped.
   No command makes a network call. This includes `motd`: system info (uptime, host, time)
   is local, and the "fun fact" list is embedded into the binary at build time
   (`include_str!`). If any future optional feature ever needs the network, it must degrade
   to a clean message (e.g. `No internet connection`), never an ugly error or stack trace.

3. **Restore is never destructive.** Before `restore` overwrites the working folder, it
   automatically takes a safety snapshot of the current state. A restore is itself just a
   new point in history, so "I went back too far" is always recoverable.

4. **Restore is an exact mirror of the tracked set.** `restore <n>` makes the tracked files
   match snapshot n exactly: files created after n are deleted (recoverable via the safety
   snapshot from Rule 3). Restore NEVER touches ignored paths (`.git`, `node_modules`,
   `target`, …) — they were never tracked, so they are never deleted or modified.

5. **Commit write order is fixed (crash safety).** A commit writes: all blobs first →
   then the manifest (only once every blob is durably on disk) → then `HEAD` last. Every
   write is atomic (write-then-rename). If interrupted, the worst case is orphan blobs
   (harmless), never a manifest that references a missing blob.

6. **Writers take the lock.** `commit` and `restore` acquire `.historia/lock` (PID +
   timestamp) before doing work and release it after. If the lock is held, the command
   fails fast with a clean message and a non-zero exit code (fail-fast, no waiting). A
   clearly stale lock is reported to the user rather than silently ignored.

7. **Round-trip test is part of "done."** From CP6 onward, every checkpoint's definition of
   done includes: `commit` → mutate → `restore` → the tracked files are **content-identical**
   to the snapshot, and `verify` passes. ("Content-identical", not bit-for-bit metadata:
   file mode is platform-dependent — Windows has no POSIX exec bit.)

8. **The on-disk format is the future-proof contract (see §9).** Keep it boring, simple,
   documented, and versioned. Data must be recoverable by hand or a short script even if
   this binary and Rust itself disappear.

9. **Minimal dependencies.** Every crate is a future breakage point. Justify each one in §7
   before adding it.

10. **No giant files.** One responsibility per file (see §8). Adding a command must not
    require touching unrelated code.

11. **Stream large files.** Never read a whole file into memory to hash or copy it; hash and
    copy in chunks. A 116 MB file must not cost 116 MB of RAM.

---

## 4. Planning methodology (default for this project)

Applied to this project and to anything new we build here, especially when Claude Code
does the implementation:

- **MVP first.** Get a usable, testable version working early so bugs surface while they
  are cheap to fix.
- **Build strictly on top of the previous step.** Each checkpoint leaves the tool working
  and committed.
- **Hardest features last.** Order steps so the riskiest work comes at the very end. If the
  last step fails, the result is still a complete, usable program — the hardest failure
  cannot sink the project. This is graceful degradation, not a promise of zero bugs.
- **Each checkpoint is independently working and tested.** "Done" means demoable + tested,
  not "code written."
- **No abstraction before a second real use case.** Extract an interface only when a second
  case proves its shape.

---

## 5. Command surface (MVP)

Canonical form on the left; aliases in parentheses. Syntax mirrors git where safe.

```
historia init [dir]              # create store (.historia/) — defaults to current dir
historia commit -m "msg"         # snapshot the whole folder            (alias: snapshot)
historia log                     # list snapshots: number, time, message
historia status                  # what changed since the last snapshot
historia restore <n>             # restore whole folder to snapshot <n> (exact mirror)
historia restore <n> <path>      # restore a single file from snapshot <n>
historia verify                  # re-hash all blobs, check store integrity
historia help [command]          # list commands, or detail one   (aliases: -h, --help, man, info)
historia --version               # version
```

### init targeting (git semantics, no surprises)
- `historia init`      → current folder
- `historia init .`    → current folder (explicit)
- `historia init ..`   → parent folder
- `historia init path` → that path (created if missing)

All other commands locate `.historia/` by walking up from the current directory, so they
work from a subfolder too — exactly like git.

### commit behaviour
- **Skip-if-unchanged (default):** if the working folder matches `HEAD` exactly, no snapshot
  is created. Prints a clean message ("nothing to snapshot, working folder matches snapshot N")
  and returns a success exit code (so a scheduler does not see it as an error).
- `--allow-empty` forces a snapshot even when nothing changed (for manual milestones).
- The "does the working folder match HEAD?" comparison is shared with `status` (one function).

### restore behaviour
- Exact mirror of the tracked set (Rule 4) + automatic safety snapshot first (Rule 3).
- Ignored paths are never touched.

### symlinks
- `commit` does not follow or store symlinks. It prints a warning per skipped link
  (`skipped symlink: <path>`) so nothing is silently lost. (Storing links as strings is a
  possible later extension; deferred because Windows-native symlink creation needs elevated
  privileges and would make restore fail on some machines.)

### Deliberate non-features (do not add)
- **No `add` / no staging.** `commit` always captures the whole folder as-is. An `add`
  command would promise selection that does not exist. Scope is controlled permanently via
  `.historiaignore`, not per-commit. `add` is intentionally absent, permanently.
- **`restore` is NOT aliased to `reset`/`checkout`.** Those carry destructive git muscle
  memory. historia's restore is non-destructive; a distinct name avoids a false friend.
- No branch / merge / remote / rebase.

### Snapshot identity
Sequential integers (1, 2, 3, …), shown first in `log`. Content hashes exist internally but
are not the primary handle — `historia restore 14` beats `restore a3f9c2` for one linear history.

### Metadata captured (MVP)
Path + content + executable bit. No ACLs, no ownership. Exec bit is best-effort and
platform-dependent (see Rule 7). Extend only if a real need proves it.

### Default ignores (overridable via .historiaignore)
`.git`, `node_modules`, `target`, `dist`, `build`. Plus `.historia/` itself, always.

---

## 6. Scheduling compatibility (design constraint from day one)

Built-in scheduling is NOT in the MVP, but the tool must be compatible with it from the
start: every command is non-interactive and returns clean exit codes, so an external
scheduler can run e.g. `historia commit -m "auto"` daily without any interactive prompt.
The skip-if-unchanged + fail-fast-lock + success-exit-on-noop decisions above exist
specifically so unattended scheduled runs stay clean. Never add a required interactive
prompt to a core command.

---

## 7. Dependencies (justify each; keep minimal)

| Crate                 | Purpose                               | Stage |
|-----------------------|---------------------------------------|-------|
| `clap`                | arg parsing, subcommands, help        | MVP |
| `sha2`                | content hashing (SHA-256)             | MVP (`blake3` optional later for speed) |
| `ignore`              | `.historiaignore` + default ignores   | MVP (ripgrep's matcher) |
| `walkdir`             | folder traversal                      | MVP (may be covered by `ignore`) |
| `serde` / `serde_json`| manifest read/write                   | MVP (JSON = human-readable, future-proof) |
| `ed25519-dalek`       | snapshot signing                      | LATE (cyber) |
| `age`                 | encrypted backups                     | LATE (cyber) |

Record MSRV and Rust edition (2021) in `Cargo.toml` and here. Prefer std over a crate when
the std path is not materially worse.

---

## 8. Directory structure

Thin entry point; engine separated from CLI; one file per command. A Rust dev should find
"what does commit do" in `commands/commit.rs` and "how are blobs stored" in `core/store.rs`
immediately.

```
historia/
├── Cargo.toml            # deps, edition 2021, MSRV; release profile with crt-static
├── Cargo.lock
├── README.md
├── CLAUDE.md             # this file
├── LICENSE               # MIT
├── .gitignore            # /target, etc.
├── .historiaignore       # dogfood: historia ignoring its own build artifacts
├── src/
│   ├── main.rs           # thin: parse → dispatch → map errors to exit codes. No logic.
│   ├── cli/
│   │   ├── mod.rs        # command REGISTRY + alias table (add a command here, one line)
│   │   ├── args.rs       # clap definitions
│   │   └── help.rs       # help text generated from the registry
│   ├── commands/         # one file per command — thin adapters that call core/
│   │   ├── mod.rs
│   │   ├── init.rs
│   │   ├── commit.rs
│   │   ├── log.rs
│   │   ├── status.rs
│   │   ├── restore.rs
│   │   └── verify.rs
│   ├── core/             # the engine: reusable, CLI-agnostic, unit-testable (#[cfg(test)])
│   │   ├── mod.rs
│   │   ├── store.rs      # locate/open .historia; read/write blobs by hash; the lock
│   │   ├── hash.rs       # streaming content hashing
│   │   ├── snapshot.rs   # manifest read/write, snapshot numbering, HEAD
│   │   ├── ignore.rs     # default ignores + .historiaignore, via `ignore` crate
│   │   ├── walk.rs       # folder traversal (skips symlinks with a warning)
│   │   └── fsutil.rs     # atomic write-then-rename; safe (mirror) restore
│   └── format/
│       └── manifest.rs   # on-disk schema + format version (the future-proof contract)
└── tests/
    ├── round_trip.rs     # integration: commit → mutate → restore → content-identical → verify
    └── fixtures/         # test data
```

### How to add a command (the customizability goal)
1. Create `commands/<name>.rs` with a `run(...)` function that calls into `core/`.
2. Register it in `cli/mod.rs`: name + aliases + one-line help.
Nothing else changes. This is why the tool is easy to extend without breaking anything.

---

## 9. On-disk format (the durability contract)

Deliberately boring and documented, so data survives even if this binary and Rust do not.

```
.historia/
├── objects/            # content-addressed blobs, sharded by hash prefix
│   └── ab/cdef1234...  # file = raw bytes of some file version; name = its SHA-256
├── snapshots/
│   ├── 1.json          # manifest per snapshot
│   ├── 2.json
│   └── ...
├── HEAD                # current snapshot number (plain text)
├── lock                # present only during a commit/restore (PID + timestamp)
└── format              # format version marker, e.g. "historia format v1"
```

**Manifest (JSON):** `{ number, timestamp, message, parent, entries: [ { path, hash, mode } ] }`
- `hash` points into `objects/`. Content addressing gives dedup + integrity for free:
  unchanged files across snapshots share one blob.
- `parent` is the previous snapshot number now; from CP13 it also carries the parent
  manifest's hash to form a tamper-evident chain.
- `mode` is stored as a DECIMAL integer of the Unix permission bits (e.g. 420 = 0o644).
  It is best-effort and platform-dependent (Rule 7): on Windows it is a sensible default,
  not a real POSIX bit. Restore applies it best-effort; round-trip equality is defined on
  content, not on mode.

**Write order (Rule 5):** blobs → manifest → HEAD, each via write-then-rename. The object
store is append-only; never mutate a blob in place. Restore writes into the working folder
only after the safety snapshot.

---

## 10. Step-by-step checkpoint plan

Ordered easy→hard. CP0–CP9 are the MVP: by the end of CP8 the tool is a fully usable, tested
versioned backup tool. CP10+ are enhancements; the cyber trio (CP13–15) is genuinely last so
its failure cannot sink the tool.

Each checkpoint ends with: Claude Code prints the suggested commit message and stops; the
author commits and pushes (Rule 1).

### MVP

- [x] **CP0 — Scaffolding & dispatch.** Cargo project (edition 2021, MSRV recorded), module
  skeleton per §8, clap wired, command registry in `cli/mod.rs`, release profile with
  `crt-static`. Generate README.md, LICENSE (MIT), .gitignore, .historiaignore. Working:
  `historia --version` and a `help` stub listing commands.
  *Done when:* both commands run; `cargo build` clean; structure matches §8.

- [x] **CP1 — `init`.** Create `.historia/` with `objects/`, `snapshots/`, `HEAD`, `format`.
  Support `init [dir]` per §5. Refuse politely if a store already exists.
  *Done when:* store is created correctly from all four init forms.

- [x] **CP2 — Store, hashing & lock (internal).** `core/hash.rs` (streaming), `core/store.rs`
  (write/read a blob by hash, atomic write-then-rename), and the `.historia/lock` primitive
  (acquire/release, fail-fast, stale detection). Unit-tested, no user command yet.
  *Done when:* unit tests cover write→read round-trip, dedup of identical content, and that a
  second process cannot take a held lock.

- [x] **CP3 — `commit` / `snapshot`.** Walk the folder (skip symlinks + warn; respect §5
  default ignores), hash each file, take the lock, write blobs → manifest → HEAD in that
  order, release the lock. Implement skip-if-unchanged (+ `--allow-empty`).
  *Done when:* `commit -m` produces a manifest; every entry has a blob; committing an
  unchanged folder adds no snapshot and exits success; dedup verified programmatically.

- [x] **CP4 — `log`.** List snapshots: number, timestamp, message (newest first).
  *Done when:* output matches the committed manifests.

- [x] **CP5 — `status`.** Compare working folder to `HEAD`: added / modified / deleted,
  respecting ignores. Shares the comparison function with skip-if-unchanged.
  *Done when:* correct across all three change types on a test folder.

- [x] **CP6 — `restore` (mirror + safety snapshot + lock).** `restore <n>` for the whole
  folder (exact mirror of the tracked set, Rule 4) and `restore <n> <path>` for one file.
  Take a safety snapshot first (Rule 3); take the lock (Rule 6); never touch ignored paths.
  *Done when:* **round-trip test passes** (Rule 7) — commit, mutate, restore, tracked files
  content-identical to the snapshot; pre-restore state recoverable as a new snapshot; ignored
  paths untouched.

- [x] **CP7 — `.historiaignore`.** Parse it via the `ignore` crate, layered on the default
  ignores. Document precedence.
  *Done when:* patterns include/exclude correctly; defaults still apply; `.historia/` always ignored.

- [x] **CP8 — `verify`.** Re-hash every blob against its name; validate every manifest
  reference resolves. Report corruption/tampering clearly.
  *Done when:* passes on a good store; detects a deliberately corrupted blob.

- [ ] **CP9 — Help polish.** `help <command>` detail, `man`/`info` aliases, `-h`/`--help`.
  *Done when:* every command and alias is documented in-tool.

> ▲ End of MVP. The tool is usable and safe from here. Everything below is optional hardening.

### Enhancements (hardest last)

- [ ] **CP10 — Release packaging.** `crt-static` dependency-free Windows `.exe`; document
  Windows + Linux builds; GitHub Releases artifact. Install = download one file, put on PATH.
  (Author's dev install stays `cargo install --path .`.)
  *Done when:* a fresh machine with no Rust runs the downloaded binary.

- [ ] **CP11 — `backup <path>`.** Copy the `.historia/` store to another local path. (NAS is a
  future target; do not abstract the destination until a second real destination exists.)
  *Done when:* the copied store passes `verify` at the destination.

- [ ] **CP12 — `motd`.** Offline only: uptime, host, time + a fun fact from an embedded list
  (`include_str!`). Optional figlet-style banner. Never touches the network.
  *Done when:* runs identically with networking disabled.

- [ ] **CP13 — Hash chain (tamper-evident history).** Each manifest records the parent
  manifest's hash; `verify` walks and validates the chain.
  *Done when:* altering any past manifest makes `verify` fail at the right point.

- [ ] **CP14 — Signing (Ed25519).** Sign snapshots; `verify` checks signatures. Local keys.
  (Key storage/protection/loss is its own small design discussion — decide when CP14 nears.)
  *Done when:* a tampered or unsigned snapshot is flagged.

- [ ] **CP15 — Encrypted backups (age).** Encrypt the backup at rest (builds on CP11), so it
  is safe to push to untrusted storage / a future NAS.
  *Done when:* an encrypted backup round-trips: decrypt → `verify` passes.

---

## 11. Deferred / future ideas

- **Garbage collection / `prune`.** Every version of every file is kept forever; the store
  grows without bound. This is an accepted operational constraint for now, NOT a silent
  surprise. A `gc`/`prune` command (drop snapshots older than N / unreachable blobs) is the
  eventual answer. Deferred until store size actually becomes a problem.
- Built-in scheduling (currently external via Task Scheduler/cron — §6 keeps this open).
- NAS backup destination (needs real hardware first).
- Chunking / delta storage (only if a large, frequently-changing binary proves the need).
- Symlinks stored as strings (needs the Windows-privilege question resolved).
- Richer metadata (ACLs, ownership) — only if a real use case appears.
- `diff` between snapshots.
- Wait-with-timeout lock mode (if manual concurrent use becomes annoying).

---

## 12. Locked decisions

- Language: Rust. Single binary, no runtime, crate fit, novelty. (§2)
- Not git: different model, safer restore, no nested-.git problem, learning + control. (§2)
- License: MIT.
- Offline / air-gapped always; no runtime network, motd included. (Rule 2)
- Author writes and pushes every commit; Claude Code only suggests the message. (Rule 1)
- init separate from first commit (git semantics).
- Metadata = path + content + exec bit (exec bit best-effort, platform-dependent).
- mode stored as decimal Unix permission bits; best-effort; content-only round-trip equality.
- Default ignores on; overridable.
- restore = exact mirror of tracked set; never touches ignored paths; safety snapshot first.
- commit = skip-if-unchanged by default; `--allow-empty` to force.
- Concurrency = fail-fast lock (`.historia/lock`, PID + timestamp).
- symlinks = skip + warn.
- Tests = light (unit + one round-trip integration test), round-trip mandatory from CP6.
- Write order = blobs → manifest → HEAD, all atomic.