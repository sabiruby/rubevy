# Compiled to mod.mrb by tools/compile_scripts.sh.
# A mod, in the second VM (`RubevyPlugin::<Mods>::for_vm("assets/mods")`). Everything
# it tries here is something a mod should not be able to do to the game — and one
# thing it should: reading a file of its own.

# 1. the game's global and the game's class
seen = 0.0
seen += 1.0 if $world_seed
begin
  Boss
  seen += 2.0
rescue StandardError
end
Rubevy.ask("peek", seen)

# 2. the game's file, by the same name the game's own script requires
begin
  require "helper"
  Rubevy.ask("require", "read the game's helper: #{Helper.greet("mod")}")
rescue Exception => e                 # a LoadError is not a StandardError
  Rubevy.ask("require", e.class.to_s)
end

# 3. its own file, from its own root, which does load
begin
  require "mod_helper"
  Rubevy.ask("own_require", ModHelper.name_of_the_mod)
rescue Exception => e
  Rubevy.ask("own_require", "failed: #{e.class}")
end

# and now the runaway: a loop that never sleeps and never ends. The game's frame is
# not the mod's to spend, so this costs the mod its own budget and nothing else.
n = 0
loop do
  n += 1
end
