// Copyright (c) Aptos
// SPDX-License-Identifier: Apache-2.0

use crate::{
    sparse_merkle::node::{NodeHandle, NodeInner, SubTree},
    test_utils::{naive_smt::NaiveSmt, proof_reader::ProofReader},
    SparseMerkleTree,
};
use aptos_crypto::{hash::SPARSE_MERKLE_PLACEHOLDER_HASH, HashValue};
use aptos_types::state_store::state_value::StateValue;
use proptest::{
    collection::{hash_set, vec},
    prelude::*,
    sample::Index,
};
use std::{collections::VecDeque, sync::Arc};

type TxnOutput = Vec<(HashValue, StateValue)>;
type BlockOutput = Vec<TxnOutput>;

#[derive(Debug)]
pub enum Action {
    Commit,
    Execute(BlockOutput),
}

pub fn arb_smt_correctness_case() -> impl Strategy<Value = Vec<Action>> {
    (
        hash_set(any::<HashValue>(), 1..100), // keys
        vec(
            prop_oneof![
                vec(
                    // txns
                    vec(
                        // txn updates
                        (any::<Index>(), any::<Vec<u8>>()),
                        1..4,
                    ),
                    1..10,
                ),
                Just(vec![]),
            ],
            1..10,
        ),
    )
        .prop_map(|(keys, commit_or_execute)| {
            let keys: Vec<_> = keys.into_iter().collect();
            commit_or_execute
                .into_iter()
                .map(|txns| {
                    if txns.is_empty() {
                        Action::Commit
                    } else {
                        Action::Execute(
                            txns.into_iter()
                                .map(|updates| {
                                    updates
                                        .into_iter()
                                        .map(|(k_idx, v)| (*k_idx.get(&keys), v.to_vec().into()))
                                        .collect()
                                })
                                .collect::<Vec<_>>(),
                        )
                    }
                })
                .collect::<Vec<_>>()
        })
}

pub fn test_smt_correctness_impl(input: Vec<Action>) {
    let mut naive_q = VecDeque::new();
    naive_q.push_back(NaiveSmt::new::<StateValue>(&[]));
    let mut updater_q = VecDeque::new();
    updater_q.push_back(SparseMerkleTree::new(*SPARSE_MERKLE_PLACEHOLDER_HASH));

    for action in input {
        match action {
            Action::Commit => {
                if naive_q.len() > 1 {
                    naive_q.pop_front();
                    updater_q.pop_front();
                }
            }
            Action::Execute(block) => {
                let updates = block
                    .iter()
                    .map(|txn_updates| txn_updates.iter().map(|(k, v)| (*k, v)).collect())
                    .collect::<Vec<Vec<_>>>();
                let updates_flat_batch = updates.iter().flatten().cloned().collect::<Vec<_>>();

                let committed = naive_q.front_mut().unwrap();
                let proofs = updates_flat_batch
                    .iter()
                    .map(|(k, _)| (*k, committed.get_proof(k)))
                    .collect();
                let proof_reader = ProofReader::new(proofs);

                let mut naive_smt = naive_q.back().unwrap().clone().update(&updates_flat_batch);

                let updater_smt = updater_q
                    .back()
                    .unwrap()
                    .batch_update(updates_flat_batch, &proof_reader)
                    .unwrap();
                updater_q.back().unwrap().assert_no_external_strong_ref();

                assert_eq!(updater_smt.root_hash(), naive_smt.get_root_hash());

                naive_q.push_back(naive_smt);
                updater_q.push_back(updater_smt);
            }
        }
    }
}

trait AssertNoExternalStrongRef {
    fn assert_no_external_strong_ref(&self);
}

impl<V> AssertNoExternalStrongRef for SparseMerkleTree<V> {
    fn assert_no_external_strong_ref(&self) {
        assert_subtree_sole_strong_ref(&self.inner.root);
    }
}

fn assert_subtree_sole_strong_ref<V>(subtree: &SubTree<V>) {
    if let SubTree::NonEmpty {
        root: NodeHandle::Shared(arc),
        ..
    } = subtree
    {
        assert_eq!(Arc::strong_count(arc), 1);
        if let NodeInner::Internal(internal_node) = arc.inner() {
            assert_subtree_sole_strong_ref(&internal_node.left);
            assert_subtree_sole_strong_ref(&internal_node.right);
        }
    }
}
