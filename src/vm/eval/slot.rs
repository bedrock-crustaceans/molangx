//! The type parameter of the interpreter: the slot type.

use super::Vm;
use crate::vm::{cx::EvalCx, host::Host, value::Value};

/// What a value is to one instantiation of the interpreter.
pub(super) trait Slot<H: Host>: Clone + Sized {
    fn float(x: f32) -> Self;
    /// [`Value::as_f32`].
    fn f32(&self) -> f32;
    /// `None` when this instantiation cannot hold `value` (deoptimise).
    fn from_ref(value: &Value<H>) -> Option<Self>;
    fn to_value(&self) -> Value<H>;
    fn equals(a: &Self, b: &Self, cx: &EvalCx<'_, '_, H>) -> bool;
    /// Whether the payload's 64 bits equal `hash`; a float's are its 32 bits zero-extended
    /// ([`Value::molang_eq`]).
    fn is_hash(&self, hash: u64) -> bool;
    /// `Some` only on the general instantiation.
    fn general(vm: &mut Vm<H, Self>) -> Option<&mut Vm<H, Value<H>>>;
    /// [`EvalCx::storable`].
    fn storable(self, cx: &EvalCx<'_, '_, H>) -> Self;
    /// [`Value::store_cost`].
    fn store_cost(&self) -> u64;
}

impl<H: Host> Slot<H> for f32 {
    #[inline]
    fn float(x: f32) -> Self {
        x
    }

    #[inline]
    fn f32(&self) -> f32 {
        *self
    }

    #[inline]
    fn from_ref(value: &Value<H>) -> Option<Self> {
        match value {
            Value::Float(x) => Some(*x),
            _ => None,
        }
    }

    #[inline]
    fn to_value(&self) -> Value<H> {
        Value::Float(*self)
    }

    #[inline]
    fn equals(a: &Self, b: &Self, _cx: &EvalCx<'_, '_, H>) -> bool {
        a == b
    }

    #[inline]
    fn is_hash(&self, hash: u64) -> bool {
        u64::from(self.to_bits()) == hash
    }

    #[inline]
    fn general(_vm: &mut Vm<H, Self>) -> Option<&mut Vm<H, Value<H>>> {
        None
    }

    #[inline]
    fn storable(self, _cx: &EvalCx<'_, '_, H>) -> Self {
        self
    }

    #[inline]
    fn store_cost(&self) -> u64 {
        0
    }
}

impl<H: Host> Slot<H> for Value<H> {
    #[inline]
    fn float(x: f32) -> Self {
        Value::Float(x)
    }

    #[inline]
    fn f32(&self) -> f32 {
        self.as_f32()
    }

    #[inline]
    fn from_ref(value: &Value<H>) -> Option<Self> {
        Some(value.clone())
    }

    #[inline]
    fn to_value(&self) -> Value<H> {
        self.clone()
    }

    fn equals(a: &Self, b: &Self, cx: &EvalCx<'_, '_, H>) -> bool {
        a.molang_eq(b, |actor| cx.resolve_actor(actor))
    }

    #[inline]
    fn is_hash(&self, hash: u64) -> bool {
        match self {
            Value::Hash(h) => h.as_u64() == hash,
            Value::Float(x) => u64::from(x.to_bits()) == hash,
            _ => false,
        }
    }

    #[inline]
    fn general(vm: &mut Vm<H, Self>) -> Option<&mut Vm<H, Value<H>>> {
        Some(vm)
    }

    #[inline]
    fn storable(self, cx: &EvalCx<'_, '_, H>) -> Self {
        cx.storable(self)
    }

    #[inline]
    fn store_cost(&self) -> u64 {
        Value::store_cost(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::program::Instr;
    use crate::hash::HashedStr;
    use crate::vm::eval::test_support::*;
    use crate::vm::{NoHost, NoHostEnv};

    #[test]
    fn slot_is_hash_compares_the_payload_bits() {
        let h = HashedStr::new("moo");
        let bits = 1.5_f32.to_bits();
        assert!(<f32 as Slot<NoHost>>::is_hash(&1.5, u64::from(bits)));
        assert!(!<f32 as Slot<NoHost>>::is_hash(
            &1.5,
            u64::from(bits) | 1 << 32
        ));
        assert!(<f32 as Slot<NoHost>>::is_hash(&0.0, 0));
        assert!(!<f32 as Slot<NoHost>>::is_hash(&-0.0, 0));
        assert!(NV::Hash(h).is_hash(h.as_u64()));
        assert!(!NV::Hash(h).is_hash(h.as_u64() ^ 1));
        assert!(NV::Float(1.5).is_hash(u64::from(bits)));
        assert!(!NV::Float(1.5).is_hash(h.as_u64()));
        assert!(!NV::Item(()).is_hash(0));
        assert!(!NV::Actor(()).is_hash(0));
    }

    #[test]
    fn slot_conversions() {
        assert_eq!(<f32 as Slot<NoHost>>::from_ref(&NV::Float(2.0)), Some(2.0));
        assert_eq!(<f32 as Slot<NoHost>>::from_ref(&NV::string("a")), None);
        assert_eq!(<f32 as Slot<NoHost>>::from_ref(&NV::Actor(())), None);
        assert_eq!(
            <NV as Slot<NoHost>>::from_ref(&NV::string("a")),
            Some(NV::string("a"))
        );
        assert_eq!(<f32 as Slot<NoHost>>::to_value(&3.0), NV::Float(3.0));
        assert_eq!(
            <NV as Slot<NoHost>>::to_value(&NV::string("a")),
            NV::string("a")
        );
        assert_eq!(
            <NV as Slot<NoHost>>::f32(&NV::string("moo")).to_bits(),
            NV::string("moo").as_f32().to_bits()
        );
        assert_eq!(<NV as Slot<NoHost>>::float(1.0), NV::Float(1.0));
        assert_eq!(<f32 as Slot<NoHost>>::float(1.0), 1.0);
    }

    #[test]
    fn only_the_general_slot_has_a_general_interpreter() {
        let program = asm(vec![Instr::End]);
        let mut float = Vm::<NoHost, f32>::new(&program, Some(1));
        assert!(<f32 as Slot<NoHost>>::general(&mut float).is_none());
        let mut general = Vm::<NoHost, NV>::new(&program, Some(1));
        assert!(<NV as Slot<NoHost>>::general(&mut general).is_some());
    }

    #[test]
    fn slot_store_cost_is_the_actor_array_length() {
        assert_eq!(<f32 as Slot<NoHost>>::store_cost(&1.0), 0);
        assert_eq!(<NV as Slot<NoHost>>::store_cost(&NV::Float(1.0)), 0);
        assert_eq!(<NV as Slot<NoHost>>::store_cost(&NV::string("a")), 0);
        assert_eq!(
            <NV as Slot<NoHost>>::store_cost(&NV::actor_array([(), (), ()])),
            3
        );
    }

    #[test]
    fn slot_equals_compares_floats_and_values() {
        let mut env = NoHostEnv::new();
        let cx = env.cx();
        assert!(<f32 as Slot<NoHost>>::equals(&1.0, &1.0, &cx));
        assert!(!<f32 as Slot<NoHost>>::equals(&1.0, &2.0, &cx));
        assert!(!<f32 as Slot<NoHost>>::equals(&f32::NAN, &f32::NAN, &cx));
        assert!(<f32 as Slot<NoHost>>::equals(&0.0, &-0.0, &cx));
        assert!(<NV as Slot<NoHost>>::equals(
            &NV::string("a"),
            &NV::string("a"),
            &cx
        ));
        assert!(!<NV as Slot<NoHost>>::equals(
            &NV::string("a"),
            &NV::string("b"),
            &cx
        ));
        assert!(<NV as Slot<NoHost>>::equals(
            &NV::Float(2.0),
            &NV::Float(2.0),
            &cx
        ));
    }
}
