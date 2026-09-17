# rubevy documents

The layout every repository of the organization uses is described in
[sabiruby/.github/CONTRIBUTING.md](https://github.com/sabiruby/.github/blob/main/CONTRIBUTING.md).
This repository is small, so its documents sit flat here; the worklog has its own directory.

| file | language | what it is |
|---|---|---|
| [host-api.md](host-api.md) | English | what a script can say to the game and the game to a script: `Rubevy.ask`, where the game's systems go in the frame (`RubevySet`), what a question may carry (`Arg`, including a Hash or an Array), answers (including an object of the game's own), entities, components by name, events, `Rubevy::Proxy`, adding to the VM at `Startup`, time limits and pausing, running a second VM for mods under a name tag, replacing a script |
| [rust-bridge.ja.md](rust-bridge.ja.md) | Japanese | how a robot's question travels through rubevy and the game and back, point by point against embedding the C mruby |
| [outlook.md](outlook.md) | English | what rubevy builds on SabiRuby, in order, with the status of each item; the honest comparison with Lua; the possibilities |
| [outlook.ja.md](outlook.ja.md) | Japanese | the possibilities and the status, in plain Japanese |
| [plans/ecs-bridge-plan.md](plans/ecs-bridge-plan.md) | Japanese | the instructions for the ECS bridge (components by name through reflection) and the event queue; status inside |
| [plans/multi-vm-plan.md](plans/multi-vm-plan.md) | Japanese | the instructions for more than one VM in one app: what was already per-VM, the six places that assumed one, three designs (the type marker chosen), the stages; the result is in `host-api.md` and `examples/two_vms.rs` |
| [worklog/](worklog/) | Japanese | dated records of work: what was read, tried, decided |

The worklog, newest last:

| file | what it records |
|---|---|
| [worklog/2026-09-15-host-state-and-data.md](worklog/2026-09-15-host-state-and-data.md) | the command queue moved into the VM's host state, and entities as Data objects |
| [worklog/2026-09-15-stage3b-no-internal-access.md](worklog/2026-09-15-stage3b-no-internal-access.md) | the four places that reached into `Vm`'s fields, moved to the VM's own entry points |
| [worklog/2026-09-15-stage6bc-futures-proxy.md](worklog/2026-09-15-stage6bc-futures-proxy.md) | `answer_with` (a request answered from a future), and why the dynamic proxy of stage 6c stopped at the VM's C-function boundary |
| [worklog/2026-09-15-stage6c-proxy.md](worklog/2026-09-15-stage6c-proxy.md) | the dynamic proxy finished once the VM stopped putting a boundary around `method_missing`: what `proxy.rb` is, and what it does not say about itself |
| [worklog/2026-09-15-ecs-bridge.md](worklog/2026-09-15-ecs-bridge.md) | components by name through Bevy's reflection: what the plan assumed about `Vec3` and `TransformPlugin`, and why the read is `get` and not `[]` |
| [worklog/2026-09-15-events.md](worklog/2026-09-15-events.md) | events as queues: where a subscription is let go of, the 64-message limit, and the two tests that were racing the schedule |
| [worklog/2026-09-15-entity-index.md](worklog/2026-09-15-entity-index.md) | `e[:Transform]` once the VM stopped putting a boundary around `OP_GETIDX`: what changed in sabiruby, and why `get` stayed |
| [worklog/2026-09-16-arg-value.md](worklog/2026-09-16-arg-value.md) | a Hash or an Array as an argument of `Rubevy.ask`: why the value travels rather than a copy of it, and the two-step release that a `Drop` without a `Vm` needs |
| [worklog/2026-09-16-bridge-followups.md](worklog/2026-09-16-bridge-followups.md) | the three the bridge left behind: `funcall` replaced by the VM's own entry points, a `Task.new` task carrying the script's entity, and the closed queue that ends a waiting task |
| [worklog/2026-09-17-scheduling-and-vm-access.md](worklog/2026-09-17-scheduling-and-vm-access.md) | `RubevySet` (where a game's answering system goes, and why two frames was luck rather than a rule), a pause that does not spend a `sleep`, and the two things the next game needed that turned out to be there already |
| [worklog/2026-09-17-restart-burst.md](worklog/2026-09-17-restart-burst.md) | ten scripts replaced in one frame froze the whole VM: what rubevy's own way of closing a subscription had to do with it, the sixty lost frames that matched ten creatures' sixty reflex tasks exactly, and why the fix belonged in the VM |
| [worklog/2026-09-17-multi-vm.md](worklog/2026-09-17-multi-vm.md) | a name tag on the VM, and then a second VM behind it (the whole multi-VM plan): why a default type parameter is filled in where a type is written but not where a value is, what that cost `RubevySet` (an enum no longer), which derives really add bounds to a generic, the component hook that took `::<M>` after all, the two tests that failed for the right reasons — a script restarted by the very removal meant to end it, and a guard whose absence nothing could see until an asset was added between the two plugins — why `publish` could not have crossed VMs even if it had wanted to, and the mod in `examples/two_vms.rs` that spins three million instructions without costing the game a beat |
