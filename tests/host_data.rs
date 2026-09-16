//! A game answers a question with an **object of its own**: a Rust value that stays in Rust,
//! which Ruby holds a `Data` handle to.
//!
//! `ScriptWorld::answer_value` hands the host the `&mut Vm`, so everything sabiruby's
//! `#[derive(RubyClass)]` needs is already reachable — `Genome { … }.into_ruby(vm)` puts the
//! value in the VM's own `HostStore` and answers the object naming it. Nothing had to be added
//! to rubevy for this; what the test is for is that the two host mechanisms do not collide.
//! rubevy uses `Vm::set_host_state` (one value, its command queue) and `Vm::set_on_free` (its
//! own hook, counting `Rubevy::Entity`s); a `RubyClass` uses a store found by `TypeId` and the
//! VM's own drop. They are different places, and the collection at the end of this file is what
//! says so.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, RubevyPlugin, RubevySet, Script, ScriptWorld};
use sabiruby::host_store::RubyClass;
use sabiruby::{IntoRuby, RubyClass as DeriveRubyClass, ruby_methods};

/// The game's own type. Ruby never sees these numbers, only the object.
#[derive(DeriveRubyClass)]
struct Genome {
    speed: f64,
    colour: String,
}

#[ruby_methods]
impl Genome {
    fn speed(&self) -> f64 {
        self.speed
    }
    fn colour(&self) -> String {
        self.colour.clone()
    }
    fn mutate(&mut self, by: f64) {
        self.speed += by;
    }
}

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

#[derive(Resource, Default)]
struct Said(Vec<String>);

/// The methods of the class are registered once, at `Startup` (`docs/host-api.md`, "Adding to
/// the VM"). The class and the store would appear by themselves on the first `into_ruby`; what
/// `register` adds is the methods.
fn install(mut world: ResMut<ScriptWorld>) {
    Genome::register(&mut world.vm).expect("the class registers");
}

/// `answer_value` gets the `&mut Vm`, so the answer is built with the same call a host outside
/// Bevy would use.
fn answer_requests(mut world: ResMut<ScriptWorld>, mut said: ResMut<Said>) {
    for request in world.take_requests() {
        match request.kind.as_str() {
            "genome" => world.answer_value(&request, |vm| {
                Genome { speed: 2.5, colour: String::from("blue") }.into_ruby(vm)
            }),
            _ => {
                if let Some(text) = request.text(0) {
                    said.0.push(text.to_string());
                }
                world.answer(&request, Answer::Nil);
            }
        }
    }
}

/// The object comes back from `pop` like any other answer, and it is the game's class with the
/// game's methods on it — including one that changes the Rust value in place.
const USES_GENOME: &str = r#"
  g = Rubevy.ask("genome").pop
  Rubevy.ask("said", g.class.to_s)
  Rubevy.ask("said", g.colour)
  Rubevy.ask("said", g.speed.to_s)
  g.mutate(0.5)
  Rubevy.ask("said", g.speed.to_s)
  $kept = nil
"#;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Said>()
    .add_systems(Startup, install)
    .add_systems(Update, answer_requests.in_set(RubevySet::Answer));
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(2));
        app.update();
    }
}

#[test]
fn a_game_answers_with_an_object_of_its_own() {
    let mut app = app();
    let script = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(USES_GENOME));
    app.world_mut().spawn(Script::new(script));
    frames(&mut app, 12);

    let said = &app.world().resource::<Said>().0;
    assert_eq!(said, &vec!["Genome", "blue", "2.5", "3.0"], "what the script saw: {said:?}");

    // one value in the game's store, and it is rubevy's `Rubevy::Entity` hook that did not fire
    // for it: the two host mechanisms are in different places
    let world = &mut *app.world_mut().resource_mut::<ScriptWorld>();
    assert_eq!(Genome::store(&mut world.vm).len(), 1, "the Genome is in the VM's store");
    assert_eq!(world.freed_entities(), 0, "rubevy's own free hook is not involved");
}

/// And it is collected like anything else: when the script has ended and nothing names the
/// object, the `Genome` is dropped out of the store. That is the VM's doing, not rubevy's — it
/// is what makes a `RubyClass` safe to hand out from an answer.
#[test]
fn the_value_is_dropped_when_the_object_is_collected() {
    let mut app = app();
    let script = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(USES_GENOME));
    app.world_mut().spawn(Script::new(script));
    frames(&mut app, 12);

    let world = &mut *app.world_mut().resource_mut::<ScriptWorld>();
    assert_eq!(Genome::store(&mut world.vm).len(), 1);
    // the script has run to its end, so its frame is gone and `$kept` is nil; nothing names the
    // object any more
    world.vm.gc_collect();
    assert_eq!(Genome::store(&mut world.vm).len(), 0, "the Genome went with its Ruby object");
}
