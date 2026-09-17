# A question the *game* answers, and still no frame goes by: the game registered `nearest` with
# `ScriptWorld::answer_in_tick`, so the closure is called between two runs of the VM with the
# world as it stands in RubevySet::Tick. `weather` is the same `Rubevy.ask` answered the usual
# way, by a system in RubevySet::Answer — one frame, every time. See examples/nearest.rs.

me = Rubevy.entity

5.times do
  f0 = $rubevy[:frame]
  plant = Rubevy.ask("nearest", me).pop           # answered inside this tick
  at = plant ? plant[:Transform][:translation] : nil   # and so is the read that follows it
  Rubevy.log "nearest: frame #{f0}: the nearest plant is #{plant.inspect} at " \
             "#{at[0].round(1)}, #{at[2].round(1)} (frames waited: #{$rubevy[:frame] - f0})"

  f1 = $rubevy[:frame]
  sky = Rubevy.ask("weather").pop                 # answered by a system, at the end of the frame
  Rubevy.log "nearest: frame #{f1}: the weather is #{sky} " \
             "(frames waited: #{$rubevy[:frame] - f1})"
  sleep 0.05
end

:nearest_done
