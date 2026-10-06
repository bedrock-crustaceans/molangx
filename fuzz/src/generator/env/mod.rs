//! The world the generated programs run in: a few actors, variable maps holding every kind of
//! value, a context, the queries the generator calls, and the differential check of one
//! [`FuzzCase`](super::FuzzCase).

mod compare;
mod differential;
mod table;

pub use compare::{same_entries, same_temps, same_value, state_difference};
pub use differential::{Verdict, differential};
pub use table::queries;

use arbitrary::{Arbitrary, Unstructured};
use molangx::rng::{
    FixedRng, Xorshift128,
    rand_core::{Infallible, TryRng},
};
use molangx::vm::{
    CollectSink, ContextMap, ContextName, EvalCx, EvalLimits, Host, HostAccess, QueryTable,
    StructValue, Subjects, TempMap, Temps, Value, VariableMap, VariableName, VariableStore,
};

/// The host marker of the fuzz world.
#[derive(Debug)]
pub struct FuzzHost;

/// An actor handle: a direct handle (`Handle(0)` is null) or a unique id.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum FuzzActor {
    /// A direct handle.
    Handle(u8),
    /// A unique id.
    Id(u8),
}

impl FuzzActor {
    /// The actor number.
    pub const fn number(self) -> u8 {
        match self {
            Self::Handle(n) | Self::Id(n) => n,
        }
    }
}

/// The number of actor slots (actor 0 is the null handle).
pub const ACTORS: usize = 5;

impl Host for FuzzHost {
    type ActorRef = FuzzActor;
    type ItemRef = u8;
    type BlockRef = ();
    type Access<'w> = FuzzWorld;
}

/// Which actors are alive and which are babies, as bit masks over the actor numbers.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct FuzzWorld {
    /// Live actors.
    pub alive: u8,
    /// Actors with the baby flag.
    pub baby: u8,
}

impl FuzzWorld {
    fn is_alive(self, n: u8) -> bool {
        n != 0 && usize::from(n) < ACTORS && self.alive & (1 << n) != 0
    }
}

impl HostAccess<FuzzHost> for FuzzWorld {
    fn resolve_actor(&self, from: &Subjects<FuzzHost>, actor: FuzzActor) -> Option<FuzzActor> {
        match actor {
            FuzzActor::Handle(n) => self.is_alive(n).then_some(FuzzActor::Handle(n)),
            // An id resolves through the current subject's actor.
            FuzzActor::Id(n) => {
                (from.actor.is_some() && self.is_alive(n)).then_some(FuzzActor::Handle(n))
            }
        }
    }

    fn subjects_of(&self, actor: FuzzActor) -> Subjects<FuzzHost> {
        Subjects {
            this: f32::from(actor.number()),
            ..Subjects::actor(actor)
        }
    }

    fn stored_actor(&self, actor: FuzzActor) -> FuzzActor {
        FuzzActor::Id(actor.number())
    }
}

/// `variable.*`: one map per actor slot and a detached map.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FuzzVars {
    /// The detached map (no actor subject).
    pub local: VariableMap<FuzzHost>,
    /// The map of each actor number.
    pub actors: [VariableMap<FuzzHost>; ACTORS],
}

impl FuzzVars {
    fn map(&self, actor: FuzzActor) -> &VariableMap<FuzzHost> {
        &self.actors[usize::from(actor.number()) % ACTORS]
    }

    fn map_mut(&mut self, actor: FuzzActor) -> &mut VariableMap<FuzzHost> {
        &mut self.actors[usize::from(actor.number()) % ACTORS]
    }
}

impl VariableStore<FuzzHost> for FuzzVars {
    fn get(&self, actor: FuzzActor, name: VariableName) -> Option<&Value<FuzzHost>> {
        self.map(actor).get(name)
    }

    fn set(&mut self, actor: FuzzActor, name: VariableName, value: Value<FuzzHost>) {
        self.map_mut(actor).set(name, value);
    }

    fn get_public(&self, actor: FuzzActor, name: VariableName) -> Option<&Value<FuzzHost>> {
        self.map(actor).get_public(name)
    }

    fn get_local(&self, name: VariableName) -> Option<&Value<FuzzHost>> {
        self.local.get(name)
    }

    fn set_local(&mut self, name: VariableName, value: Value<FuzzHost>) {
        self.local.set(name, value);
    }
}

/// The random source of a run: xorshift or a forced word; comparable afterwards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FuzzRng {
    /// xorshift128 from the standard seeds.
    Xorshift(Xorshift128),
    /// The same word every time.
    Fixed(FixedRng),
}

impl TryRng for FuzzRng {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        match self {
            Self::Xorshift(rng) => rng.try_next_u32(),
            Self::Fixed(rng) => rng.try_next_u32(),
        }
    }

    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        match self {
            Self::Xorshift(rng) => rng.try_next_u64(),
            Self::Fixed(rng) => rng.try_next_u64(),
        }
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Infallible> {
        match self {
            Self::Xorshift(rng) => rng.try_fill_bytes(dst),
            Self::Fixed(rng) => rng.try_fill_bytes(dst),
        }
    }
}

/// Whom an expression runs for.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Subject {
    /// Actor 1, whose variable map is `variable.*`.
    Actor,
    /// No subject: `variable.*` is the detached map.
    Detached,
}

/// How long `temp.*` lives.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TempLifetime {
    /// Fresh temps for every evaluation.
    PerEvaluation,
    /// A temp map the host keeps across evaluations.
    Persistent,
}

impl<'a> Arbitrary<'a> for Subject {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(if u.arbitrary()? {
            Self::Actor
        } else {
            Self::Detached
        })
    }
}

impl<'a> Arbitrary<'a> for TempLifetime {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(if u.arbitrary()? {
            Self::Persistent
        } else {
            Self::PerEvaluation
        })
    }
}

/// Everything one run owns.
#[derive(Clone, Debug)]
pub struct FuzzEnv {
    /// `variable.*`.
    pub vars: FuzzVars,
    /// `context.*`.
    pub context: ContextMap<FuzzHost>,
    /// `temp.*` kept across evaluations, or `None` for fresh temps every evaluation.
    pub temps: Option<TempMap<FuzzHost>>,
    /// The actors.
    pub world: FuzzWorld,
    /// The queries.
    pub queries: QueryTable<FuzzHost>,
    /// The subject actor, if any.
    pub subject: Option<FuzzActor>,
    /// `this`.
    pub this: f32,
    /// The budgets.
    pub limits: EvalLimits,
    /// The random source.
    pub rng: FuzzRng,
    /// The collected run-time messages.
    pub sink: CollectSink,
}

fn var(name: &str) -> VariableName {
    VariableName::new(name)
}

impl FuzzEnv {
    /// The starting state of every run: actors 1–3 alive (2 a baby), 4 removed; the subject's map
    /// and the detached map hold a float, a negative, a NaN, a string, a struct, an actor, an actor
    /// array; actors 2 and 3 have public variables with snapshots; the context has an actor, a
    /// number and an actor array.
    pub fn new(subject: Subject, temps: TempLifetime, limits: EvalLimits, rng: FuzzRng) -> Self {
        let mut vars = FuzzVars::default();
        let structure = StructValue::from([
            ("x", Value::Float(1.0)),
            ("y", Value::structure(StructValue::from([("z", 2.0)]))),
        ]);
        let array = Value::actor_array([
            FuzzActor::Handle(1),
            FuzzActor::Handle(4),
            FuzzActor::Id(3),
            FuzzActor::Handle(0),
        ]);
        for map in [&mut vars.local, &mut vars.actors[1]] {
            map.set(var("x"), Value::Float(1.5));
            map.set(var("y"), Value::Float(-3.0));
            map.set(var("n"), Value::Float(f32::NAN));
            map.set(var("s"), Value::string("moo"));
            map.set(var("st"), Value::structure(structure.clone()));
            map.set(var("e"), Value::Actor(FuzzActor::Id(2)));
            map.set(var("arr"), array.clone());
            map.set(var("a"), Value::Float(0.0));
        }
        vars.actors[2].set_public(var("x"), Value::Float(7.0));
        vars.actors[2].set(var("y"), Value::Float(3.0));
        vars.actors[2].set(var("e"), Value::Actor(FuzzActor::Id(1)));
        vars.actors[2].refresh_snapshots();
        vars.actors[2].set_public(var("x"), Value::Float(8.0));
        vars.actors[3].set_public(var("x"), Value::Float(9.0));
        vars.actors[3].refresh_snapshots();
        let context = ContextMap::from([
            (
                ContextName::new("other"),
                Value::Actor(FuzzActor::Handle(2)),
            ),
            (ContextName::new("n"), Value::Float(4.0)),
            (
                ContextName::new("arr"),
                Value::actor_array([
                    FuzzActor::Handle(2),
                    FuzzActor::Handle(3),
                    FuzzActor::Handle(4),
                ]),
            ),
        ]);
        Self {
            vars,
            context,
            temps: match temps {
                TempLifetime::PerEvaluation => None,
                TempLifetime::Persistent => Some(TempMap::new()),
            },
            world: FuzzWorld {
                alive: 0b1110,
                baby: 0b0100,
            },
            queries: queries(),
            subject: match subject {
                Subject::Actor => Some(FuzzActor::Handle(1)),
                Subject::Detached => None,
            },
            this: 2.34,
            limits,
            rng,
            sink: CollectSink::new(),
        }
    }

    /// The evaluation context over this environment.
    pub fn cx(&mut self) -> EvalCx<'_, 'static, FuzzHost> {
        let subjects = match self.subject {
            Some(actor) => Subjects::actor(actor),
            None => Subjects::none(),
        };
        EvalCx {
            subjects: Subjects {
                this: self.this,
                ..subjects
            },
            host: &mut self.world,
            variables: &mut self.vars,
            context: &self.context,
            queries: Some(&self.queries),
            rng: &mut self.rng,
            sink: &mut self.sink,
            limits: self.limits,
            temps: match &mut self.temps {
                Some(temps) => Temps::Kept(temps),
                None => Temps::PerEvaluation,
            },
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::generator::env::{FuzzEnv, FuzzHost, FuzzRng, Subject, TempLifetime};
    use molangx::rng::{FixedRng, Xorshift128};
    use molangx::vm::{EvalLimits, Value, VariableName};

    /// Always the sample 0.25.
    pub(crate) const QUARTER: FixedRng = FixedRng(1 << 29);

    pub(crate) type V = Value<FuzzHost>;

    /// Actor 1 for subject, fresh temps, the default budgets and the sample 0.5.
    pub(crate) fn env() -> FuzzEnv {
        FuzzEnv::new(
            Subject::Actor,
            TempLifetime::PerEvaluation,
            EvalLimits::DEFAULT,
            FuzzRng::Fixed(FixedRng::HALF),
        )
    }

    /// No subject actor, fresh temps, the default budgets and the sample 0.5.
    pub(crate) fn detached_env() -> FuzzEnv {
        FuzzEnv::new(
            Subject::Detached,
            TempLifetime::PerEvaluation,
            EvalLimits::DEFAULT,
            FuzzRng::Fixed(FixedRng::HALF),
        )
    }

    pub(crate) fn xorshift_env() -> FuzzEnv {
        FuzzEnv::new(
            Subject::Actor,
            TempLifetime::PerEvaluation,
            EvalLimits::DEFAULT,
            FuzzRng::Xorshift(Xorshift128::new()),
        )
    }

    pub(crate) fn var(name: &str) -> VariableName {
        VariableName::new(name)
    }

    pub(crate) fn float(x: f32) -> V {
        Value::Float(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::env::test_support::{var, *};
    use molangx::hash::HashedStr;
    use molangx::rng::sample;
    use molangx::vm::{Access, ContextProvider};

    #[test]
    fn an_actor_number_is_the_same_for_a_handle_and_an_id() {
        assert_eq!(FuzzActor::Handle(3).number(), 3);
        assert_eq!(FuzzActor::Id(3).number(), 3);
        assert_eq!(FuzzActor::Handle(0).number(), 0);
        assert_eq!(FuzzActor::Id(255).number(), 255);
        assert_ne!(FuzzActor::Handle(3), FuzzActor::Id(3));
        assert_eq!(ACTORS, 5);
    }

    #[test]
    fn the_host_marker_uses_actor_handles_and_small_item_handles() {
        fn actor_ref<H: Host<ActorRef = FuzzActor, ItemRef = u8, BlockRef = ()>>() {}
        actor_ref::<FuzzHost>();
    }

    #[test]
    fn an_actor_is_alive_when_it_is_in_range_non_null_and_its_bit_is_set() {
        let all = FuzzWorld {
            alive: 0xff,
            baby: 0,
        };
        assert!(!all.is_alive(0), "actor 0 is the null handle");
        for n in 1..=4 {
            assert!(all.is_alive(n), "actor {n}");
        }
        for n in 5..=255 {
            assert!(!all.is_alive(n), "actor {n} is past the slots");
        }
        let some = FuzzWorld {
            alive: 0b0100,
            baby: 0,
        };
        assert!(some.is_alive(2));
        assert!(!some.is_alive(1) && !some.is_alive(3) && !some.is_alive(4));
        let none = FuzzWorld::default();
        assert!((0..=255).all(|n| !none.is_alive(n)));
        // Bit 0 alone does not make the null handle alive.
        assert!(!FuzzWorld { alive: 1, baby: 0 }.is_alive(0));
    }

    #[test]
    fn a_direct_handle_resolves_when_its_actor_is_alive_whoever_asks() {
        let world = FuzzWorld {
            alive: 0b1110,
            baby: 0,
        };
        for from in [Subjects::none(), Subjects::actor(FuzzActor::Handle(2))] {
            assert_eq!(
                world.resolve_actor(&from, FuzzActor::Handle(1)),
                Some(FuzzActor::Handle(1))
            );
            assert_eq!(
                world.resolve_actor(&from, FuzzActor::Handle(3)),
                Some(FuzzActor::Handle(3))
            );
            assert_eq!(world.resolve_actor(&from, FuzzActor::Handle(0)), None);
            assert_eq!(
                world.resolve_actor(&from, FuzzActor::Handle(4)),
                None,
                "bit 4 is clear"
            );
            assert_eq!(world.resolve_actor(&from, FuzzActor::Handle(9)), None);
        }
    }

    #[test]
    fn an_id_resolves_to_a_direct_handle_only_from_a_subject_with_an_actor() {
        let world = FuzzWorld {
            alive: 0b1110,
            baby: 0,
        };
        assert_eq!(
            world.resolve_actor(&Subjects::none(), FuzzActor::Id(2)),
            None,
            "no subject actor: no resolution"
        );
        let from = Subjects::actor(FuzzActor::Handle(1));
        assert_eq!(
            world.resolve_actor(&from, FuzzActor::Id(2)),
            Some(FuzzActor::Handle(2)),
            "an id comes out as a direct handle"
        );
        assert_eq!(world.resolve_actor(&from, FuzzActor::Id(4)), None);
        assert_eq!(world.resolve_actor(&from, FuzzActor::Id(0)), None);
        // The subject's own liveness does not matter, only that it has an actor.
        let dead = Subjects::actor(FuzzActor::Handle(4));
        assert_eq!(
            world.resolve_actor(&dead, FuzzActor::Id(3)),
            Some(FuzzActor::Handle(3))
        );
    }

    #[test]
    fn the_subjects_of_an_actor_carry_its_number_as_this() {
        let world = FuzzWorld::default();
        assert_eq!(
            world.subjects_of(FuzzActor::Handle(3)),
            Subjects {
                this: 3.0,
                ..Subjects::actor(FuzzActor::Handle(3))
            }
        );
        assert_eq!(
            world.subjects_of(FuzzActor::Id(2)),
            Subjects {
                this: 2.0,
                ..Subjects::actor(FuzzActor::Id(2))
            }
        );
        assert_eq!(world.subjects_of(FuzzActor::Handle(0)).this, 0.0);
    }

    #[test]
    fn a_stored_actor_is_its_id() {
        let world = FuzzWorld::default();
        assert_eq!(world.stored_actor(FuzzActor::Handle(3)), FuzzActor::Id(3));
        assert_eq!(world.stored_actor(FuzzActor::Id(3)), FuzzActor::Id(3));
        assert_eq!(world.stored_actor(FuzzActor::Handle(0)), FuzzActor::Id(0));
    }

    #[test]
    fn an_actor_number_picks_its_map_modulo_the_slots() {
        let mut vars = FuzzVars::default();
        vars.set(FuzzActor::Handle(6), var("k"), float(1.0));
        // 6 % 5 == 1: actor 1 sees the write of actor 6, and the slot is the map of index 1.
        assert_eq!(vars.get(FuzzActor::Handle(1), var("k")), Some(&float(1.0)));
        assert_eq!(vars.get(FuzzActor::Id(11), var("k")), Some(&float(1.0)));
        assert_eq!(vars.actors[1].len(), 1);
        assert!(vars.actors[0].is_empty() && vars.actors[2].is_empty());
        assert_eq!(vars.get(FuzzActor::Handle(2), var("k")), None);
        assert!(vars.local.is_empty(), "the detached map is a separate one");
    }

    #[test]
    fn the_detached_map_is_used_by_local_reads_and_writes_only() {
        let mut vars = FuzzVars::default();
        vars.set_local(var("l"), float(2.0));
        assert_eq!(vars.get_local(var("l")), Some(&float(2.0)));
        assert_eq!(vars.get(FuzzActor::Handle(0), var("l")), None);
        assert!(vars.actors.iter().all(VariableMap::is_empty));
        vars.set(FuzzActor::Handle(0), var("a0"), float(3.0));
        assert_eq!(vars.get_local(var("a0")), None);
        assert_eq!(vars.actors[0].len(), 1);
    }

    #[test]
    fn a_public_read_sees_the_snapshot_of_the_actors_slot() {
        let mut vars = FuzzVars::default();
        vars.actors[2].set_public(var("p"), float(1.0));
        assert_eq!(
            vars.get_public(FuzzActor::Handle(2), var("p")),
            None,
            "no snapshot yet"
        );
        vars.actors[2].refresh_snapshots();
        vars.actors[2].set_public(var("p"), float(2.0));
        assert_eq!(
            vars.get_public(FuzzActor::Handle(2), var("p")),
            Some(&float(1.0))
        );
        assert_eq!(vars.get(FuzzActor::Handle(2), var("p")), Some(&float(2.0)));
        assert_eq!(
            vars.get_public(FuzzActor::Id(7), var("p")),
            Some(&float(1.0)),
            "7 % 5 == 2"
        );
        assert_eq!(vars.get_public(FuzzActor::Handle(1), var("p")), None);
    }

    #[test]
    fn a_write_keeps_the_access_of_an_existing_slot() {
        let mut vars = FuzzVars::default();
        vars.actors[1].set_public(var("p"), float(1.0));
        vars.set(FuzzActor::Handle(1), var("p"), float(5.0));
        assert_eq!(vars.actors[1].access(var("p")), Some(Access::Public));
        vars.set(FuzzActor::Handle(1), var("fresh"), float(5.0));
        assert_eq!(vars.actors[1].access(var("fresh")), Some(Access::Private));
    }

    #[test]
    fn a_fixed_source_gives_its_sample_every_time() {
        let mut rng = FuzzRng::Fixed(QUARTER);
        for _ in 0..5 {
            assert_eq!(sample(&mut rng), 0.25);
        }
        assert_eq!(
            rng,
            FuzzRng::Fixed(QUARTER),
            "drawing leaves a fixed source as it was"
        );
    }

    #[test]
    fn the_xorshift_source_draws_what_a_fresh_generator_draws() {
        let mut rng = FuzzRng::Xorshift(Xorshift128::new());
        let mut reference = Xorshift128::new();
        for _ in 0..10 {
            assert_eq!(sample(&mut rng).to_bits(), sample(&mut reference).to_bits());
        }
        assert_eq!(rng, FuzzRng::Xorshift(reference));
        assert_ne!(
            rng,
            FuzzRng::Xorshift(Xorshift128::new()),
            "ten draws moved the state"
        );
    }

    #[test]
    fn sources_compare_by_state_and_a_fixed_source_by_its_word() {
        assert_ne!(
            FuzzRng::Fixed(FixedRng(1)),
            FuzzRng::Fixed(FixedRng(0x8000_0001)),
            "the same sample, another word"
        );
        assert_eq!(
            FuzzRng::Fixed(FixedRng::HALF),
            FuzzRng::Fixed(FixedRng::HALF)
        );
        assert_ne!(FuzzRng::Fixed(FixedRng::HALF), FuzzRng::Fixed(QUARTER));
        assert_eq!(
            FuzzRng::Xorshift(Xorshift128::new()),
            FuzzRng::Xorshift(Xorshift128::new())
        );
        assert_ne!(
            FuzzRng::Fixed(FixedRng::HALF),
            FuzzRng::Xorshift(Xorshift128::new())
        );
        assert_ne!(
            FuzzRng::Xorshift(Xorshift128::new()),
            FuzzRng::Fixed(FixedRng::HALF)
        );
    }

    #[test]
    fn every_method_of_the_source_draws_from_its_generator() {
        use molangx::rng::rand_core::Rng;
        let mut rng = FuzzRng::Xorshift(Xorshift128::new());
        let mut reference = Xorshift128::new();
        assert_eq!(rng.next_u64(), reference.next_u64());
        let (mut got, mut want) = ([0_u8; 5], [0_u8; 5]);
        rng.fill_bytes(&mut got);
        reference.fill_bytes(&mut want);
        assert_eq!((got, rng), (want, FuzzRng::Xorshift(reference)));
        let mut fixed = FuzzRng::Fixed(FixedRng(0x0102_0304));
        assert_eq!(fixed.next_u64(), 0x0102_0304_0102_0304);
        fixed.fill_bytes(&mut got);
        assert_eq!(got, [4, 3, 2, 1, 4]);
    }

    #[test]
    fn the_world_has_three_live_actors_one_of_them_a_baby_and_one_removed() {
        let env = env();
        assert_eq!(
            env.world,
            FuzzWorld {
                alive: 0b1110,
                baby: 0b0100
            }
        );
        assert!(env.world.is_alive(1) && env.world.is_alive(2) && env.world.is_alive(3));
        assert!(!env.world.is_alive(4), "actor 4 is removed");
        assert!(!env.world.is_alive(0));
        assert_eq!(env.this, 2.34);
        assert!(env.sink.messages.is_empty());
    }

    #[test]
    fn the_subject_is_actor_one_or_nothing() {
        assert_eq!(
            FuzzEnv::new(
                Subject::Actor,
                TempLifetime::PerEvaluation,
                EvalLimits::DEFAULT,
                FuzzRng::Fixed(FixedRng::HALF)
            )
            .subject,
            Some(FuzzActor::Handle(1))
        );
        assert_eq!(detached_env().subject, None);
    }

    #[test]
    fn temps_are_kept_by_the_host_only_for_the_persistent_lifetime() {
        assert!(
            FuzzEnv::new(
                Subject::Actor,
                TempLifetime::PerEvaluation,
                EvalLimits::DEFAULT,
                FuzzRng::Fixed(FixedRng::HALF)
            )
            .temps
            .is_none()
        );
        let persistent = FuzzEnv::new(
            Subject::Actor,
            TempLifetime::Persistent,
            EvalLimits::DEFAULT,
            FuzzRng::Fixed(FixedRng::HALF),
        );
        assert!(persistent.temps.as_ref().is_some_and(TempMap::is_empty));
    }

    #[test]
    fn the_limits_and_the_random_source_are_the_ones_given() {
        let limits = EvalLimits {
            loop_iterations: Some(3),
            total_steps: Some(77),
            struct_depth: Some(4),
            struct_members: Some(5),
            query_depth: Some(6),
        };
        let env = FuzzEnv::new(
            Subject::Detached,
            TempLifetime::PerEvaluation,
            limits,
            FuzzRng::Fixed(FixedRng(1 << 28)),
        );
        assert_eq!(env.limits, limits);
        assert_eq!(env.rng, FuzzRng::Fixed(FixedRng(1 << 28)));
    }

    #[test]
    fn the_subject_and_the_detached_map_hold_the_same_eight_variables() {
        let env = env();
        let structure = StructValue::from([
            ("x", Value::Float(1.0)),
            ("y", Value::structure(StructValue::from([("z", 2.0)]))),
        ]);
        let array = Value::actor_array([
            FuzzActor::Handle(1),
            FuzzActor::Handle(4),
            FuzzActor::Id(3),
            FuzzActor::Handle(0),
        ]);
        for (which, map) in [
            ("detached", &env.vars.local),
            ("actor 1", &env.vars.actors[1]),
        ] {
            assert_eq!(map.len(), 8, "{which}");
            assert_eq!(map.get(var("x")), Some(&float(1.5)), "{which}");
            assert_eq!(map.get(var("y")), Some(&float(-3.0)), "{which}");
            assert!(
                matches!(map.get(var("n")), Some(Value::Float(x)) if x.is_nan()),
                "{which}"
            );
            assert_eq!(map.get(var("s")), Some(&Value::string("moo")), "{which}");
            assert_eq!(
                map.get(var("st")),
                Some(&Value::structure(structure.clone())),
                "{which}"
            );
            assert_eq!(
                map.get(var("e")),
                Some(&Value::Actor(FuzzActor::Id(2))),
                "{which}"
            );
            assert_eq!(map.get(var("arr")), Some(&array), "{which}");
            assert_eq!(map.get(var("a")), Some(&float(0.0)), "{which}");
            assert_eq!(map.get(var("never")), None, "{which}");
            assert_eq!(map.get(var("b")), None, "{which}");
            for name in ["x", "y", "n", "s", "st", "e", "arr", "a"] {
                assert_eq!(
                    map.access(var(name)),
                    Some(Access::Private),
                    "{which} {name}"
                );
                assert_eq!(map.get_public(var(name)), None, "{which} {name}");
            }
        }
    }

    #[test]
    fn the_structure_variable_holds_x_and_a_nested_y_z() {
        let env = env();
        let st = env.vars.local.get(var("st")).expect("st");
        assert_eq!(st.member(HashedStr::new("x")), Some(&float(1.0)));
        assert_eq!(
            st.member_path(&[HashedStr::new("y"), HashedStr::new("z")]),
            Some(&float(2.0))
        );
        assert_eq!(st.member(HashedStr::new("q")), None);
        assert_eq!(st.struct_depth(), 2);
    }

    #[test]
    fn actor_two_has_a_public_x_whose_snapshot_is_older_than_its_value() {
        let env = env();
        let map = &env.vars.actors[2];
        assert_eq!(map.len(), 3);
        assert_eq!(map.get(var("x")), Some(&float(8.0)), "the latest value");
        assert_eq!(map.get_public(var("x")), Some(&float(7.0)), "the snapshot");
        assert_eq!(map.access(var("x")), Some(Access::Public));
        assert_eq!(map.get(var("y")), Some(&float(3.0)));
        assert_eq!(map.get_public(var("y")), None, "y is private");
        assert_eq!(map.get(var("e")), Some(&Value::Actor(FuzzActor::Id(1))));
    }

    #[test]
    fn actor_three_has_a_public_x_with_a_matching_snapshot() {
        let env = env();
        let map = &env.vars.actors[3];
        assert_eq!(map.len(), 1);
        assert_eq!(map.get(var("x")), Some(&float(9.0)));
        assert_eq!(map.get_public(var("x")), Some(&float(9.0)));
        assert!(env.vars.actors[0].is_empty() && env.vars.actors[4].is_empty());
    }

    #[test]
    fn the_context_holds_an_actor_a_number_and_an_actor_array() {
        let env = env();
        let get = |name: &str| env.context.context(ContextName::new(name));
        assert_eq!(get("other"), Some(Value::Actor(FuzzActor::Handle(2))));
        assert_eq!(get("n"), Some(float(4.0)));
        assert_eq!(
            get("arr"),
            Some(Value::actor_array([
                FuzzActor::Handle(2),
                FuzzActor::Handle(3),
                FuzzActor::Handle(4)
            ]))
        );
        assert_eq!(get("missing"), None);
    }

    #[test]
    fn two_fresh_environments_are_the_same_state() {
        for subject in [Subject::Actor, Subject::Detached] {
            for temps in [TempLifetime::PerEvaluation, TempLifetime::Persistent] {
                let fresh = || {
                    FuzzEnv::new(
                        subject,
                        temps,
                        EvalLimits::DEFAULT,
                        FuzzRng::Xorshift(Xorshift128::new()),
                    )
                };
                let (a, b) = (fresh(), fresh());
                assert_eq!(state_difference(&a, &b), None, "{subject:?}, {temps:?}");
                assert_eq!(state_difference(&a, &a.clone()), None);
            }
        }
    }

    #[test]
    fn a_fresh_environment_holds_a_nan_that_a_plain_comparison_would_call_different() {
        // The reason the maps are compared by bits.
        let a = env();
        let b = env();
        assert_ne!(a.vars.local, b.vars.local, "map equality is false on a NaN");
        assert_eq!(compare::map_difference(&a.vars.local, &b.vars.local), None);
    }

    #[test]
    fn cx_carries_the_subject_this_the_limits_and_the_temps_of_the_environment() {
        let mut with_actor = env();
        let cx = with_actor.cx();
        assert_eq!(
            cx.subjects,
            Subjects {
                this: 2.34,
                ..Subjects::actor(FuzzActor::Handle(1))
            }
        );
        assert!(matches!(cx.temps, Temps::PerEvaluation));
        assert_eq!(cx.limits, EvalLimits::DEFAULT);
        let limits = EvalLimits {
            total_steps: Some(9),
            ..EvalLimits::DEFAULT
        };
        let mut detached = FuzzEnv::new(
            Subject::Detached,
            TempLifetime::Persistent,
            limits,
            FuzzRng::Fixed(FixedRng::HALF),
        );
        let cx = detached.cx();
        assert_eq!(
            cx.subjects,
            Subjects {
                this: 2.34,
                ..Subjects::none()
            }
        );
        assert!(matches!(cx.temps, Temps::Kept(_)));
        assert_eq!(cx.limits.total_steps, Some(9));
    }

    #[test]
    fn cx_reads_the_variables_of_the_subject_and_the_context() {
        let mut subject = env();
        let cx = subject.cx();
        assert_eq!(cx.variable(var("x")), Some(&float(1.5)));
        assert_eq!(cx.context(ContextName::new("n")), Some(float(4.0)));
        assert_eq!(
            cx.resolve_actor(FuzzActor::Id(2)),
            Some(FuzzActor::Handle(2))
        );
        assert_eq!(cx.resolve_actor(FuzzActor::Handle(4)), None);
        let mut other = detached_env();
        other.vars.local.set(var("only_local"), float(1.0));
        let cx = other.cx();
        assert_eq!(cx.variable(var("only_local")), Some(&float(1.0)));
        assert_eq!(
            cx.resolve_actor(FuzzActor::Id(2)),
            None,
            "an id needs a subject actor"
        );
    }

    #[test]
    fn cx_writes_go_to_the_slot_of_the_subject_actor() {
        let mut env = env();
        env.cx().set_variable(var("fresh"), float(7.0));
        assert_eq!(env.vars.actors[1].get(var("fresh")), Some(&float(7.0)));
        assert!(env.vars.local.get(var("fresh")).is_none());
        let mut detached = detached_env();
        detached.cx().set_variable(var("fresh"), float(8.0));
        assert_eq!(detached.vars.local.get(var("fresh")), Some(&float(8.0)));
        assert!(
            detached
                .vars
                .actors
                .iter()
                .all(|m| m.get(var("fresh")).is_none())
        );
    }
}
