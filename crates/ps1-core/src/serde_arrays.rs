//! `serde` support for arrays longer than 32 elements, which the derive does
//! not handle on its own. Use with `#[serde(with = "crate::serde_arrays")]`.
//! The array is written as a fixed-size tuple, so it costs no length prefix.

use serde::de::{Error, SeqAccess, Visitor};
use serde::ser::SerializeTuple;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::marker::PhantomData;

pub fn serialize<S, T, const N: usize>(array: &[T; N], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    let mut tuple = serializer.serialize_tuple(N)?;
    for item in array {
        tuple.serialize_element(item)?;
    }
    tuple.end()
}

pub fn deserialize<'de, D, T, const N: usize>(deserializer: D) -> Result<[T; N], D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default + Copy,
{
    struct ArrayVisitor<T, const N: usize>(PhantomData<T>);

    impl<'de, T, const N: usize> Visitor<'de> for ArrayVisitor<T, N>
    where
        T: Deserialize<'de> + Default + Copy,
    {
        type Value = [T; N];

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            write!(formatter, "an array of {N} elements")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<[T; N], A::Error> {
            let mut out = [T::default(); N];
            for (i, slot) in out.iter_mut().enumerate() {
                *slot = seq.next_element()?.ok_or_else(|| Error::invalid_length(i, &self))?;
            }
            Ok(out)
        }
    }

    deserializer.deserialize_tuple(N, ArrayVisitor(PhantomData))
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Wrapper {
        #[serde(with = "super")]
        values: [u16; 100],
    }

    #[test]
    fn long_arrays_round_trip() {
        let mut values = [0u16; 100];
        for (i, v) in values.iter_mut().enumerate() {
            *v = (i * 7) as u16;
        }
        let bytes = bincode::serialize(&Wrapper { values }).unwrap();
        assert_eq!(bytes.len(), 200, "no length prefix");
        assert_eq!(bincode::deserialize::<Wrapper>(&bytes).unwrap().values, values);
    }
}
