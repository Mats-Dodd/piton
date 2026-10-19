//! Long random walks through the real session, shrunk to a minimal failing
//! sequence when an invariant breaks.

#![cfg(not(piton_planted_bug))]

use piton_model::{WorldConfig, walk};
use proptest::{collection::vec, prelude::*};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    #[test]
    fn random_walks_keep_the_invariants(choices in vec(any::<u16>(), 0..400)) {
        let outcome = walk(&WorldConfig::deep(), &choices);
        prop_assert!(outcome.is_ok(), "{:?}", outcome.err().map(|(_, divergence)| divergence));
    }
}
