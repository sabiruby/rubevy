# rubevy's control layer: `on(:event) { |payload| … }` in a script, and a task per handler.
#
# Loaded by the app, not by rubevy (`rubevy::layers::CONTROL`, `ScriptWorld::load_and_run`), so a
# game that wants none of it has no `on` and no `Rubevy::Control`. It is Ruby over
# `Rubevy.subscribe` and `Task.new` and nothing else (docs/host-api.md, "The control layer").
#
# **What it is for.** A script that is mostly "when the game says X, do Y" — a level's rules, a
# tutorial, the part of a game a designer writes — reads as a list of handlers:
#
#   on(:clicked) { |x, y| Rubevy.log "clicked at #{x}, #{y}" }
#   on(:scored)  { |points| @total = (@total || 0) + points }
#   Rubevy::Control.run
#
# Each `on` subscribes to the name the game publishes (`ScriptWorld::publish`) and starts a task
# that waits on it. A handler is then an ordinary task: it may `Rubevy.ask(...).pop`, read a
# component, `sleep` — it parks, and the other handlers and the script go on.
#
# **Where the shape comes from.** It is the shape an embedding copied three times before it was
# made general: one task per handler, the subscription taken in the script's own task, a handler
# that raises is logged and keeps listening, and the handler tasks end when the subscription does.
# Those copies had to make every handler a method (`define_method`) and call it by name, because
# running a block with another `self` takes `instance_exec`, and up to SabiRuby 0.6 a task could
# not park inside `instance_exec`. SabiRuby 0.7 lets it (sabiruby `docs/design/wait-anywhere.md`),
# so a handler here is the block itself, run with `instance_exec` on the script's stage.
#
# **What `self` is in a handler.** The script's `Rubevy::Control` object — one per script, made the
# first time the script calls `on` — so an `@total` written in one handler is read in another, and
# is this script's own. (At the top level of a script `self` is `main`, which is one object for
# the whole VM: an instance variable there would be shared by every script.) A method the script
# `def`ines at its top level is callable from a handler, as it is from anywhere.
#
# **What a handler is given.** The payload the game published, as `pop` answers it, passed as one
# argument — so a list payload fills `|x, y|` the way any block's arguments fill from an Array,
# and `|payload|` gets the whole list.
module Rubevy
  class Control
    # The script's stage: the one its handlers run on. It is kept on the script's task (an
    # instance variable, as `@rubevy_entity` is), so two scripts in one VM have two.
    def self.current
      task = Task.current
      stage = task.instance_variable_get(:@rubevy_control)
      if stage.nil?
        stage = new
        task.instance_variable_set(:@rubevy_control, stage)
      end
      stage
    end

    # Parks the script's task for good, so that its handlers go on hearing. A script's
    # subscriptions are let go of when its task ends, and the handlers with them — so a script
    # that is only handlers ends with this. It returns when the script is stopped or replaced
    # (the task is terminated), not before.
    def self.run
      loop { sleep }
    end

    def initialize
      @rubevy_handlers = []
    end

    # The tasks this stage has started, one per `on`, in the order they were written.
    def handlers
      @rubevy_handlers.map { |h| h[1] }
    end

    # How many messages the handlers have lost because their queue was full, over every `on` of
    # this script (`Rubevy::Subscription#dropped` for each).
    def dropped
      total = 0
      @rubevy_handlers.each { |h| total += h[0].dropped }
      total
    end

    # One handler: subscribe now, in the task that called `on` (a subscription belongs to the
    # entity whose task asks, and this is the script's), then wait for it in a task of its own.
    # `limit:` is the subscription's queue limit, as `Rubevy.subscribe` takes it.
    def on(event, limit: nil, &block)
      raise ArgumentError, "on(#{event.inspect}) needs a block" if block.nil?
      name = event.to_sym
      queue = limit.nil? ? Rubevy.subscribe(name) : Rubevy.subscribe(name, limit: limit)
      stage = self
      # where to say a handler went wrong: in the author's lines, as `ScriptEnded::at` says where
      # a script did. The handler's own task starts in this file, so the script's file is read
      # here, off the task that called `on` (the outermost frame of `caller`), with the length of
      # the game's prelude the host put on that task; `Rubevy.__authors_place` then draws the
      # line with the same rule as `ScriptEnded::at` (`authors_places` in src/lib.rs).
      # `caller(0)` and not `caller`: this file is compiled without a line table, so its frames
      # are not in the backtrace at all, and `caller`'s arithmetic (the reference's, which takes
      # the frames it drops to be there) then cuts the script's own frame off and answers `[]`
      origin = caller(0)
      prelude_lines = Task.current.instance_variable_get(:@rubevy_prelude_lines) || 0
      task = Task.new(name: "on(:#{name})", priority: Task.current.priority) do
        begin
          loop do
            payload = queue.pop
            begin
              stage.instance_exec(payload, &block)
            rescue => e
              # the handler is logged and goes on hearing: one bad message does not stop the rest
              where = Rubevy.__authors_place(e.backtrace, origin, prelude_lines)
              Rubevy.log "on(:#{name}): #{e.class}: #{e.message}#{where ? " (#{where})" : ""}"
            end
          end
        rescue Rubevy::Unsubscribed
          # the script was stopped, replaced or ended, and rubevy closed the queue: the ordinary
          # end of a handler task
        end
      end
      @rubevy_handlers << [queue, task]
      task
    end
  end
end

# `on(:event) { … }` where a script writes it: at its top level, or in a method of its own. It
# goes to the stage of the task that calls it, which is the script's own task when it is called
# from the script's body.
def on(event, limit: nil, &block)
  Rubevy::Control.current.on(event, limit: limit, &block)
end
