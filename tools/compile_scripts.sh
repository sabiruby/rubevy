#!/bin/bash
# Compile the Ruby that ships with this crate to .mrb with the reference mruby (Docker image
# kishima/mruby:4.1.0-rc): the example scripts in assets/scripts and assets/mods, src/prelude.rb, which the
# plugin runs when the VM starts (`include_bytes!` in src/lib.rs), and src/layers/*.rb, the optional
# layers an app loads itself (`rubevy::layers`). Both .rb and .mrb are
# committed; run this after changing a .rb.
set -eu
cd "$(dirname "$0")/.."
compile() {
  local dir="$1" rb="$2"
  docker run --rm -v "$PWD/$dir:/w" kishima/mruby:4.1.0-rc mrbc -o "/w/$(basename "${rb%.rb}").mrb" "/w/$(basename "$rb")"
  echo "$dir/$rb -> $dir/${rb%.rb}.mrb"
}
for rb in assets/scripts/*.rb; do
  compile assets/scripts "$(basename "$rb")"
done
# the second VM's asset root in examples/two_vms.rs: a mod's files, which the game's VM
# cannot reach and which must be compiled all the same
for rb in assets/mods/*.rb; do
  compile assets/mods "$(basename "$rb")"
done
compile src prelude.rb
# the optional layers (`rubevy::layers`), which an app loads itself and most do not
for rb in src/layers/*.rb; do
  compile src/layers "$(basename "$rb")"
done
