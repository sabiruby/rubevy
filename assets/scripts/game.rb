# Compiled to game.mrb by tools/compile_scripts.sh.
# The game's own script, in the app's first VM (`RubevyPlugin::default()`).
# It defines the two things assets/mods/mod.rb will go looking for, reads one of
# its own files with `require`, and then beats once every 16 ms for ever.
require "helper"                      # assets/scripts/helper.mrb — this VM's load path

$world_seed = 1234

class Boss
  def taunt
    Helper.greet("mods")
  end
end

# the control for the mod's peek: in the VM that made them, all three are there
seen = 0.0
seen += 1.0 if $world_seed
begin
  Boss
  seen += 2.0
rescue StandardError
end
Rubevy.ask("peek", seen)
Rubevy.ask("required", Boss.new.taunt)

beat = 0.0
loop do
  beat += 1.0
  Rubevy.ask("beat", beat)
  sleep 0.016
end
