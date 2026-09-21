# rubevy-build

The build script that puts a game's Ruby into its binary, for [rubevy](https://crates.io/crates/rubevy).

A browser has no filesystem: a page is served a wasm module and whatever it fetches, and
`std::fs` answers nothing there. A game whose scripts are `.rb` files on a PC therefore needs the
same files *inside* the binary in the browser build. This writes the table of `include_str!`s (or
`include_bytes!`s) that rubevy's `EmbeddedHost` serves `require` out of.

```rust
// the whole of a game's build.rs, inside its `fn main`
rubevy_build::Embed::new("ruby").write();
```

```rust
// src/platform.rs, in the browser half
include!(concat!(env!("OUT_DIR"), "/ruby_files.rs"));
// `pub static RUBY_FILES: &[(&str, &str)]`, one pair per file:
// ("ruby/lib/helper.rb", "module Helper\n…")
```

The paths are written relative to the crate rather than as the absolute ones the build machine
had, so the file a script `require`s on a PC is the one it requires in a page, under the same
name. Which extensions to take, what to call the `static` and the file, and whether to embed
text or bytes (`.mrb`) are all settable — a game with two kinds of embedded file writes two
tables.

**This crate is std and nothing else**, on purpose. It is a build-dependency, and everything a
build-dependency names is compiled for the build machine before the crate that uses it starts;
putting this inside rubevy would build Bevy a second time for every game that embeds a script.
The two ends agree on a table of pairs, which is a type std already has.

It lives in the [rubevy repository](https://github.com/sabiruby/rubevy) as a second package
beside the crate itself, and what each release contains is
[`CHANGELOG.md`](https://github.com/sabiruby/rubevy/blob/main/CHANGELOG.md).

## License

MIT.
