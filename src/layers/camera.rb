# An optional Ruby layer: a camera, in the words a game uses about one.
#
# Compiled to camera.mrb by tools/compile_scripts.sh and carried in the binary as
# `rubevy::layers::CAMERA`. **Nothing loads it but the app** (`ScriptWorld::load_and_run`), and
# an app that does not load it has no `Rubevy::Camera` — which is the point of a layer: rubevy
# does not know what a camera is, and must not start to.
#
# Nothing here is Rust. Every line rests on what `docs/host-api.md` already documents —
# `Rubevy.find`, `e[:Transform]`, `e[:Projection] =`, `Rubevy.ask` — and the layer's whole job is
# to hide the three things that make a camera awkward to write from a script:
#
#   1. **The projection is an enum, and its variant cannot be switched from Ruby.** A write names
#      the variant the value is already in, so the layer reads it once and remembers it.
#   2. **2D and 3D zoom by different numbers, in different units, in opposite directions.** An
#      orthographic camera has `scale` (bigger = things look smaller); a perspective one has
#      `fov` in radians (wider = things look smaller). `zoom 2` means the same thing on both
#      here: twice as close, twice as big.
#   3. **A write lands at the end of the frame**, so reading the projection, multiplying it and
#      writing it back loses every `zoom` but the first in a tick. The magnification is kept
#      here, on the Ruby side, and written as an absolute value.
#
# It is ordinary Ruby in an ordinary VM, so a game that wants another camera marker or another
# way to follow reopens the class rather than asking rubevy for a setting.
module Rubevy
  class Camera
    # The components that say "this entity is a camera", in the order they are looked for. They
    # are bevy's two markers; a game whose camera carries one of its own reopens this.
    def self.markers
      [:Camera2d, :Camera3d]
    end

    # Every camera in the world, `markers` order, as `Rubevy::Camera` objects. It walks the world
    # once per marker (`Rubevy.find`), so it is for the start of a script or for an event — not
    # for every frame. Hold the object afterwards; it costs nothing to keep.
    def self.find_all
      found = []
      markers.each do |marker|
        Rubevy.find(marker).each { |e| found << attach(e, marker) }
      end
      found
    end

    # A camera object on an entity the caller already holds, its projection read
    # (`Rubevy::Camera.attach(e, :Camera3d)`).
    #
    # This and not `new` is the way to make one, and the reason is the VM: `Class#new` is a
    # native, so everything `initialize` does happens inside a C function — and a task cannot be
    # parked across one ("blocking pop cannot be called from within a C function boundary",
    # SabiRuby `docs/design/fibers.md`). Reading a component is a parking, so `initialize` cannot
    # read one and `reload` is called from out here, where the frame is an ordinary Ruby frame.
    def self.attach(entity, kind = nil)
      new(entity, kind).reload
    end

    # The first camera, or nil where there is none: the 2D one where a game has both, because
    # `markers` is in that order.
    def self.find
      find_all[0]
    end

    # `entity` is the camera's entity and `kind` the marker it was found by. **It reads nothing**
    # — see `attach`, which is `new` and then `reload`, and which is what `find` uses.
    def initialize(entity, kind = nil)
      @entity = entity
      @kind = kind
      @variant = nil
      @field = nil
      @base = nil
      @zoom = 1.0
      @at = nil
      @stamp = nil
      @follower = nil
    end

    # The entity, the marker it was found by (`:Camera2d` / `:Camera3d`), and the variant its
    # `Projection` was in when it was last read (`:Orthographic` / `:Perspective`, or nil where
    # the camera has no readable `Projection` at all — an unregistered type, or a camera without
    # one).
    attr_reader :entity, :kind, :variant

    # Reads the projection again and takes what it says as the new starting point: the variant,
    # the number inside it, and a magnification of 1.
    #
    # The layer remembers those because Ruby cannot change a variant and because a write is not
    # readable in the tick that made it. What it cannot know is the host changing the projection
    # from Rust — swapping a 2D camera for a 3D one, loading a level that sets the scale — and
    # this is how a script is told to look again.
    def reload
      shown = @entity[:Projection]
      shown = nil unless shown.is_a?(Hash)
      @variant = shown && shown.keys[0]
      @field = zoom_field
      inside = @field && shown[@variant]
      inside = inside.is_a?(Array) ? inside[0] : nil
      @base = inside.is_a?(Hash) ? inside[@field] : nil
      # a projection this layer cannot zoom — a variant it does not know, a type nobody
      # registered, a camera without one — leaves `zoom` and `magnification` answering nil rather
      # than writing something made up
      @field = nil unless @base.is_a?(Numeric)
      @zoom = 1.0
      @at = nil
      @stamp = nil
      self
    end

    # Where the camera is, as `[x, y, z]`, or nil where it has no `Transform`.
    #
    # It answers what this layer wrote this frame where it wrote one, and the world's own
    # otherwise. Both are needed: a write lands at the end of the frame, so the world would
    # answer the value from before two `pan`s in one tick — and the frame number ($rubevy[:frame],
    # which costs no round trip) is what tells "I wrote this, this frame" from "that was a frame
    # ago, and the game may have moved the camera since".
    def position
      return @at if @at && @stamp == frame_now
      tf = @entity[:Transform]
      tf && tf[:translation]
    end

    # Puts the camera at `x, y`, keeping its `z` where `z` is not given — which is what a 2D game
    # wants, since z there is the drawing order and not a place. Answers self, or nil where the
    # camera has no `Transform` to write.
    def move_to(x, y, z = nil)
      here = position
      return nil if here.nil?
      z = here[2] if z.nil?
      @at = [x, y, z]
      @stamp = frame_now
      @entity[:Transform] = { translation: @at }
      self
    end

    # Moves the camera by `dx, dy` (and `dz`, which a 2D game leaves alone). Two `pan`s in one
    # tick both count, for the reason `position` gives.
    def pan(dx, dy, dz = 0.0)
      here = position
      return nil if here.nil?
      move_to(here[0] + dx, here[1] + dy, here[2] + dz)
    end

    # How much bigger things look than they did when the projection was last read: 1.0 after
    # `reload`, 2.0 after `zoom 2`. nil where this camera has no projection this layer knows how
    # to zoom.
    #
    # It is **not** bevy's `OrthographicProjection#scale`, and the two run opposite ways: bevy's
    # is how much world fits across the window, so a smaller one is closer, while this is
    # apparent size, so a larger one is closer. That is why it is not called `scale` — the same
    # word meaning the reverse on the two sides of one boundary is a trap laid for whoever reads
    # `4.0x` beside `scale 0.25`.
    def magnification
      @field && @zoom
    end

    # `zoom 2` comes twice as close — things look twice as big — and `zoom 0.5` pulls back, on a
    # 2D camera and a 3D one alike. Answers the new magnification (what `magnification` answers),
    # or nil where there is no projection to write.
    #
    # The two cameras zoom by different numbers and the layer is what makes them mean the same:
    #
    # * **Orthographic**: `scale` is how much world fits across the window, so apparent size is
    #   `1 / scale` and `scale` is divided by the factor. `zoom 2` halves it.
    # * **Perspective**: `fov` is the angle the camera sees, in radians. What sets apparent size
    #   is not the angle but its tangent — half the window holds `tan(fov / 2)` of the world at
    #   unit distance — so it is `tan(fov / 2)` that is divided by the factor, and the new angle
    #   is `2 * atan(tan(fov / 2) / factor)`. Dividing the angle itself would be wrong by more
    #   and more as the angle grows (and could pass pi, which is no angle at all); this way any
    #   positive magnification lands somewhere in `0 < fov < pi`, because `atan` does.
    #
    # The magnification is kept here and written as an absolute value, rather than read from the
    # world and multiplied. A write is applied after this frame's scripts have run, so a read
    # after a write in the same tick answers the old value, and `zoom 2` twice in one tick would
    # otherwise come to 2 and not 4.
    def zoom(factor)
      return nil if @field.nil?
      @zoom *= factor
      write_projection
      @zoom
    end

    # What the game would have to work out from the camera's own placing and its projection —
    # where a point on the window is in the world — and so **not** something a component holds.
    # The layer asks for it and hands the queue back without waiting:
    #
    #   q = cam.world_at(mouse_x, mouse_y)
    #   there = q.pop              # only where the game answers "camera.world_at"
    #
    # The question is `Rubevy.ask("camera.world_at", camera_entity, x, y)`, and the entity is in
    # it because a game may have more than one camera. Answering is the game's — rubevy answers
    # nothing here, and **nobody answers it by default**: a `pop` on the queue of a question no
    # system answers parks that task for ever (the task stands on the queue, holding its context,
    # for as long as the VM lives). So `pop` it in a task of its own, or only where the game says
    # it answers this.
    def world_at(x, y)
      Rubevy.ask("camera.world_at", @entity, x, y)
    end

    # Follows `target` (a `Rubevy::Entity`) in a task of its own, and answers that task.
    #
    # `every` is what the task sleeps between turns, in seconds: 0 is "as often as the scripts
    # run", which is every frame the VM's clock moves. Nothing here decides how fast or how far
    # behind a camera should follow — a game that wants it smoothed writes a task of its own with
    # `move_to`, and this one is the plain case.
    #
    # It sleeps and does not wait on `Rubevy.next_frame`, although that would be exactly one move
    # a frame. Two reasons, both about what `every` is: a time all the way down (a follower that
    # waited on frames when it was handed 0 and on seconds otherwise would be two things under
    # one name), and the cheaper of the two waits — `sleep 0` is 14 instructions the VM settles
    # by itself against 62 and a round trip for `next_frame`, and a follower runs every frame for
    # as long as it follows. The two wake on the same frames at any frame rate a game runs at
    # (the clock moves in ticks of 4 ms, so only past about 250 Hz does `sleep 0` skip one). A
    # script that needs the promise rather than the habit writes its own task with
    # `Rubevy.each_frame` and `move_to`.
    #
    # `offset` is where the camera sits relative to the target: `[dx, dy]`, which leaves the
    # camera's own z alone (in 2D that is the drawing order and not a place; in 3D it is the
    # height the camera was put at), or `[dx, dy, dz]`, which puts the camera dz from the
    # target's z as well. Left out it is no offset at all — the camera's x and y become the
    # target's.
    #
    # It stops on `unfollow`, by itself when the target is gone (its `Transform` reads nil, which
    # is what a despawned entity answers), and by itself when **this camera** is gone (`move_to`
    # answers nil for the same reason). However it stops, `following?` goes back to false.
    # Following again replaces the task that was following.
    def follow(target, every = 0, offset = nil)
      unfollow
      here = position
      there = translation_of(target)
      return nil if here.nil? || there.nil?
      offset ||= [0.0, 0.0]
      camera = self
      @follower = Task.new(name: "camera.follow") do
        me = Task.current
        # `follower` and not a flag of its own: one thing says both "is this camera following"
        # and "is it *me* that is following", and a `follow` that replaces this one puts its own
        # task there, which is how this one is told to stop. The block does not run until the
        # scheduler reaches it, which is after the assignment below, so `me` is there by then.
        while camera.follower == me
          at = camera.translation_of(target)
          break if at.nil?
          z = offset[2] && at[2] + offset[2]
          break if camera.move_to(at[0] + offset[0], at[1] + offset[1], z).nil?
          sleep every
        end
        # It stopped by itself — the target is gone, or **this camera** is (a despawned entity's
        # `Transform` reads nil, so `move_to` answers nil) — and `following?` has to stop saying
        # yes. Only while this is still the camera's follower: a later `follow` must not have its
        # own task taken away by the one it replaced.
        camera.unfollow if camera.follower == me
      end
    end

    # Stops following. The task ends at its next turn — it may be asleep in `every` — so what
    # this promises is that no further move is made, not that the task is already gone.
    def unfollow
      @follower = nil
      self
    end

    # The task that is following, or nil. `follow` answers it too; this is how a script holding
    # the camera rather than the task asks, and it is what the following task itself watches.
    attr_reader :follower

    # Whether this camera is following something. It goes back to false by itself when the
    # following task stops — because the target was despawned, or because this camera was.
    def following?
      @follower ? true : false
    end

    # The writes this script made that the world would not take, of this camera's entity only
    # (`Rubevy.rejected_writes` is the whole of this script's). A camera whose `zoom` did nothing
    # says why here, one frame later:
    #
    #   cam.zoom 2
    #   sleep 0
    #   cam.rejected_writes.each { |w| Rubevy.log "#{w[:name]}: #{w[:reason]}" }
    #
    # The layer does not look at this by itself. It could not raise where the write was made —
    # that line ran a frame ago — so an exception would come out of some later call that had
    # nothing to do with it; and the one refusal the layer could expect, a write into the wrong
    # variant, is the one it is built not to make (`reload`).
    def rejected_writes
      Rubevy.rejected_writes.select { |w| w[:entity] == @entity }
    end

    # The translation of any entity, or nil where it has no `Transform` — which is also what an
    # entity that has been despawned answers. Public because the following task reads the
    # target's through it.
    def translation_of(entity)
      tf = entity[:Transform]
      tf && tf[:translation]
    end

    def inspect
      "#<Rubevy::Camera #{@entity.inspect} #{@kind} #{@variant} #{@zoom}x>"
    end

    private

    # Bevy's frame count, or nil outside a frame — a program the host runs itself
    # (`ScriptWorld::load_and_run`) at `Startup` is before the first one.
    def frame_now
      $rubevy && $rubevy[:frame]
    end

    # The field inside the current variant that zooming writes.
    def zoom_field
      case @variant
      when :Orthographic then :scale
      when :Perspective then :fov
      end
    end

    # One write, of the variant the projection is already in: the outer Hash names the variant,
    # the Array is its one positional field (both of bevy's are tuple variants of one), and the
    # inner Hash is a partial write of the projection's own struct — so `near`, `far` and the
    # rest keep their values.
    def write_projection
      value = case @variant
              when :Orthographic then @base / @zoom
              when :Perspective then 2.0 * Math.atan(Math.tan(@base / 2.0) / @zoom)
              end
      @entity[:Projection] = { @variant => [{ @field => value }] }
    end
  end
end
