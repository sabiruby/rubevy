# Compiled to prelude.mrb by tools/compile_scripts.sh (reference mrbc 4.1.0-rc) and run when the
# VM starts (`include_bytes!` in src/lib.rs), so every script has these without requiring
# anything: they belong to the plugin, as `Rubevy::Entity` does.
#
# Why Ruby and not `Vm::define_fn`. Every reading method here waits for an answer, and waiting
# is the one thing a native cannot do: `Rubevy.ask` parks the task on a queue, and the VM
# refuses a blocking `pop` inside a native ("blocking pop cannot be called from within a C
# function boundary", sabiruby src/builtins/ext_task.rs). A native would have to hand the queue
# back and let the script `pop` it, which is not what `e[:Transform]` should be. So the work
# stays on the Rust side (`answer_reflect_requests`, which the tick's answer loop calls between
# two runs of the VM; src/lib.rs) and the waiting is here — the same reason `Rubevy::Proxy` is
# Ruby.
module Rubevy
  class Entity
    # The component as a Hash of its fields, or nil where the entity has no component of that
    # type — or the type is not registered (`app.register_type::<T>()`; nothing unregistered is
    # visible from Ruby). The task is parked until the host answers, and that happens inside the
    # same tick: `tick_scripts` runs the scripts, answers the reads they parked on out of the
    # world it is holding, and runs them again — so the value is here, in the line that asked
    # for it. (A write is still applied at the end of the frame, so a read after a write in the
    # same tick answers the old value; see `set`.)
    def get(name)
      Rubevy.ask("component.get", self, name.to_s).pop
    end

    # `e[:Transform]`, which is what a script writes. mrbc folds a one-argument `[]` into
    # OP_GETIDX; the VM answers an Array, Hash or String itself and *sends* everything else in
    # the frame the call was made in, so this body is an ordinary frame and the task can be
    # parked in it until the host answers. It was `get` only (sabiruby ran OP_GETIDX through a
    # nested run loop, which is a native boundary a task cannot be parked across); `get` stays
    # because a script that spells the round trip out is easier to read than one that does not.
    def [](name)
      get(name)
    end

    # Writes are deferred, as `Rubevy.spawn` and `Rubevy.move_to` are: the value is applied
    # after this frame's scripts have run. Only the fields the Hash names are written, so
    # reading a component, changing one number and writing it back is one round trip and does
    # not undo what another script wrote to another field.
    def set(name, value)
      Rubevy.set_component(self, name.to_s, value)
      value
    end

    def []=(name, value)
      set(name, value)
    end

    # Whether the entity has that component. Unregistered types answer false.
    def has?(name)
      Rubevy.ask("component.has", self, name.to_s).pop
    end

    # The short type names of the registered components on this entity, sorted.
    def components
      Rubevy.ask("components", self).pop
    end
  end

  # Every entity that has that component, as an Array of Rubevy::Entity. It walks the whole
  # world, so it is for a lookup now and then — at the start, on an event — not for every frame.
  def self.find(name)
    ask("entities.with", name.to_s).pop
  end

  # A resource as a Hash of its fields, or nil where the world has no resource of that type, or
  # the type is not registered, or it is registered but does not say it is a resource
  # (`#[reflect(Resource)]`). The name is the type's short name, as a component's is
  # (`Rubevy.resource(:Score)`); a generic one carries its parameters and so needs quoting
  # (`Rubevy.resource("Time<Virtual>")`).
  #
  # It is the same round trip `Rubevy::Entity#[]` is, and it costs no frame for the same reason:
  # the task is parked here and the host answers inside the very tick that asked. The write
  # (`Rubevy.set_resource`) still lands at the end of the frame, so a read after a write in the
  # same tick answers the old value.
  def self.resource(name)
    ask("resource.get", name.to_s).pop
  end

  # Waits for the next frame and answers its number — the same Integer `$rubevy[:frame]` carries
  # while the line after this one runs.
  #
  # It is the one wait `sleep` cannot spell. `sleep 0` waits for the VM's clock to move, and the
  # clock moves in whole ticks of 4 ms, so what a script asks with it is a length of time that
  # happens to be about a frame at the frame rates a game runs at. This asks for the frame
  # itself: one call, one frame, however long the frame was.
  #
  #   loop do
  #     f = Rubevy.next_frame                  # woken at the head of the frame's tick
  #     Rubevy.log "frame #{f}, #{$rubevy[:delta]} s" if f % 60 == 0
  #   end
  #
  # The host wakes it before it runs anything else of that frame (`wake_next_frame` in
  # src/lib.rs), so the line after it sees that frame's `$rubevy` and writes what it writes into
  # that frame's writes. **A paused VM (`budget = 0`) is not a frame that is waited through**:
  # the frame it wakes on is the first one the scripts run in again.
  #
  # Every task that asked is woken, oldest first. Where the host's `frame_time` is too short to
  # wake them all, the rest are the first woken on the frame after — they lose a frame, they do
  # not lose their turn (`FrameStats::carried_reflect` counts them).
  def self.next_frame
    ask("frame.next").pop
  end

  # Runs the block once a frame, for ever: `Rubevy.each_frame { |dt| … }`.
  #
  # It is `next_frame` and a loop, written once here because it is what a game writes the loop
  # for. The block is handed the seconds since the previous frame and the frame's number, so a
  # block of one argument gets the delta:
  #
  #   Rubevy.each_frame do |dt|
  #     e[:Transform] = { translation: [x += 40.0 * dt, 0.0, 0.0] }
  #   end
  #
  # `break` in the block ends it, as it does in any block; nothing else does. A script whose
  # whole job is one pass a frame writes this in its own task (`Task.new { Rubevy.each_frame …
  # }`) where it also has something else to do.
  def self.each_frame
    loop do
      n = next_frame
      yield $rubevy[:delta], n
    end
  end

  # The writes **this script** made that the world would not take, as an Array of Hashes —
  # `{entity: Rubevy::Entity or nil, name: "Projection", reason: "…"}`, newest last, and empty
  # where every write landed.
  #
  # They are of one frame: the last one in which the scripts of this VM wrote anything at all.
  # The next frame that writes replaces the lot, so this is read after a write and before the
  # next. A script that waits with `Rubevy.next_frame` reads its own refusals on the frame the
  # write landed; one that waits with `sleep` may be two or three frames later (`sleep 0` waits
  # for the VM's clock to move, and it moves in ticks of 4 ms), which is why the list is held
  # until something replaces it rather than emptied every frame.
  #
  # A write is applied after this frame's scripts have run, so nothing can be answered where it
  # is made: `e[:X] = hash` gives back the hash it was handed, whatever becomes of it. The news
  # is a frame old by the time there is any news at all, and this is where it arrives:
  #
  #   cam[:Projection] = { Orthographic: [ { scale: 2.0 } ] }
  #   Rubevy.next_frame                              # the next frame, and the write has landed
  #   Rubevy.rejected_writes.each { |w| Rubevy.log "#{w[:name]}: #{w[:reason]}" }
  #
  # `entity` is what was written to (nil for a resource — `Rubevy.set_resource`), which is not
  # who wrote it: a script may write another entity's component, and it is the writer this
  # answers for. Only this script's own are here; the host sees every script's
  # (`ScriptWorld::rejected_writes`). A task with no entity at all is answered an empty Array.
  #
  # What lands here is a refusal and not a mishap: a type nobody registered, an entity without
  # that component, a variant of an enum that is not the one the value is in, a field that
  # cannot take what it was given. A write to an entity that was despawned in the meantime is
  # **not** here — there was nothing left to write to.
  def self.rejected_writes
    ask("writes.rejected").pop
  end

  # Raised in whatever is waiting on a subscription's queue when the subscription ends: the
  # script's entity was despawned, its `ScriptTask` was taken away, or its own task ran off its
  # end. Nothing will ever be published to that queue again, so a `pop` that answered would be
  # answering for ever.
  #
  # It is raised rather than answered with nil so that a waiting task *unwinds*: an `ensure` in
  # it runs, and the task ends instead of standing on a queue nobody can fill (a task the
  # scheduler keeps, holding its context, for as long as the VM lives). A script that wants to
  # end quietly rescues it.
  class Unsubscribed < StandardError; end

  # What `Rubevy.subscribe` extends the queue it answers with (one object, not the class: an
  # `Rubevy.ask` queue is an ordinary `Task::Queue` and keeps the gem's own meaning — unless the
  # game drops the question unanswered, which is `DroppedQuestion` below).
  #
  # mruby-task's `Queue#close` wakes everything parked on the queue and makes `pop` answer nil
  # from then on — the gem's way of saying "no more", and the reason rubevy closes a queue it
  # lets go of. nil is a poor thing for a script to have to notice, though: `loop { q.pop }`
  # against a closed queue never parks again, so the leaked task becomes a busy one. The
  # subscription turns that nil into the exception above. Items already in the queue when it
  # closed are popped first, so a script that was behind still sees what it missed.
  module Subscription
    def pop(*args)
      value = super
      raise Unsubscribed, "the subscription ended" if value.nil? && closed?
      value
    end

    alias shift pop
    alias deq pop

    # How many messages this subscription has lost because the queue was full when the game
    # published them, counting from the start. The oldest go first, so a count that has moved
    # since the last read says "there is a hole between what I read last and what I read now" —
    # which `pop` itself cannot say, because a dropped message leaves nothing behind.
    #
    # The limit is the VM's (`ScriptWorld::queue_limit`, 64 unless the game changed it) or this
    # subscription's own (`Rubevy.subscribe(:belt, limit: 512)`). A script that falls behind can
    # ask for a bigger queue, read more often, or simply say how much it missed:
    #
    #   loop do
    #     item = belt.pop
    #     missed = belt.dropped
    #     Rubevy.log "missed #{missed - @seen}" if missed > @seen
    #     @seen = missed
    #     handle item
    #   end
    #
    # The host writes it on this very object (`@rubevy_dropped`, `DROPPED_IVAR` in src/lib.rs),
    # which is why it is nil until the first message is dropped.
    def dropped
      n = @rubevy_dropped
      n.nil? ? 0 : n
    end
  end

  # Raised in whatever is waiting on a `Rubevy.ask` when the game lets the question go without
  # answering it — drops the `Request` rather than handing it to `ScriptWorld::answer`. Nothing
  # will ever answer that queue, so a `pop` that went on waiting would wait for as long as the VM
  # lives; this is the same news `Unsubscribed` is for a subscription, and it is raised for the
  # same reason: the waiting task unwinds, runs its `ensure`, and ends — or rescues it and goes on
  # without the answer.
  #
  #   begin
  #     found = Rubevy.ask("scan", 40.0).pop
  #   rescue Rubevy::Unanswered
  #     found = nil
  #   end
  class Unanswered < StandardError; end

  # What the host extends a dropped question's queue with, just before it closes it
  # (`ScriptWorld::release_dropped_values` in src/lib.rs). It is put on that one queue and only
  # then, so an answered question — every question, in a game that answers what it is asked —
  # is an ordinary `Task::Queue` with nothing in front of its `pop`.
  #
  # It is `__pop_try` and not `pop` that it stands in front of, because the task it is for is
  # already *inside* `pop`: mruby-task's `Task::Queue#pop` is a Ruby loop that calls `__pop_try`
  # until it gets an item, and a task parked on the queue wakes (on the close) inside that loop
  # and calls `__pop_try` again. A `pop` of the module's own would be looked up by the next
  # `pop`, not by the one already running.
  module DroppedQuestion
    def __pop_try(*args)
      raise Unanswered, "the game let this question go without answering it" if closed? && empty?
      super
    end
  end
end

# A task a script makes with `Task.new` carries the entity of the task that made it.
#
# `Rubevy.entity`, `Rubevy.ask` and `Rubevy.subscribe` all read the entity off the task the
# scheduler is running — the plugin hangs it on the script's own task as `@rubevy_entity`
# (`ENTITY_IVAR` in src/lib.rs) — and a task made out of a block has none, so without this a
# script's second task could not ask the game anything or subscribe to anything. SabiRuby's
# `Task` does not record which task made it (`TaskData` in src/builtins/ext_task.rs has no
# parent), and the one place that still knows is the call itself: inside `Task.new`,
# `Task.current` is the task doing the making. So the entity is copied here rather than looked
# up through a parent later; a task made by a task that inherited one inherits it in turn.
#
# A script may still set `@rubevy_entity` on a task itself — it is an ordinary instance
# variable — which is what a task that should act for another entity does.
class Task
  class << self
    alias __rubevy_plain_new new

    def new(*args, **kw, &block)
      task = __rubevy_plain_new(*args, **kw, &block)
      parent = Task.current
      entity = parent && parent.instance_variable_get(:@rubevy_entity)
      task.instance_variable_set(:@rubevy_entity, entity) unless entity.nil?
      task
    end
  end
end
