#![allow(missing_docs)]

use alloy_primitives::{hex, B256, U256};
use proptest::{prelude::ProptestConfig, proptest};
use proptest_arbitrary_interop::arb;
use reth_db::{tables, Database};
use reth_db_api::{cursor::DbCursorRW, transaction::DbTxMut};
use reth_primitives::StorageEntry;
use reth_provider::{test_utils::create_test_provider_factory, ProviderError};
use reth_trie::{
    node_iter::{TrieElement, TrieNodeIter},
    prefix_set::{PrefixSetMut, TriePrefixSets},
    trie_cursor::{InMemoryTrieCursorFactory, TrieCursor, TrieCursorFactory},
    updates::TrieUpdates,
    walker::TrieWalker,
    HashBuilder, StorageTrieEntry,
};
use reth_trie_common::{BranchNodeCompact, Nibbles};
use reth_trie_db::{
    DatabaseAccountTrieCursor, DatabaseHashedStorageCursor, DatabaseStorageTrieCursor,
    DatabaseTrieCursorFactory,
};
use std::sync::Arc;

mod common;
use common::{insert_account, State};

#[test]
fn walk_nodes_with_common_prefix() {
    let inputs = vec![
        (vec![0x5u8], BranchNodeCompact::new(0b1_0000_0101, 0b1_0000_0100, 0, vec![], None)),
        (vec![0x5u8, 0x2, 0xC], BranchNodeCompact::new(0b1000_0111, 0, 0, vec![], None)),
        (vec![0x5u8, 0x8], BranchNodeCompact::new(0b0110, 0b0100, 0, vec![], None)),
    ];
    let expected = vec![
        vec![0x5, 0x0],
        // The [0x5, 0x2] prefix is shared by the first 2 nodes, however:
        // 1. 0x2 for the first node points to the child node path
        // 2. 0x2 for the second node is a key.
        // So to proceed to add 1 and 3, we need to push the sibling first (0xC).
        vec![0x5, 0x2],
        vec![0x5, 0x2, 0xC, 0x0],
        vec![0x5, 0x2, 0xC, 0x1],
        vec![0x5, 0x2, 0xC, 0x2],
        vec![0x5, 0x2, 0xC, 0x7],
        vec![0x5, 0x8],
        vec![0x5, 0x8, 0x1],
        vec![0x5, 0x8, 0x2],
    ];

    let factory = create_test_provider_factory();
    let tx = factory.provider_rw().unwrap();

    let mut account_cursor = tx.tx_ref().cursor_write::<tables::AccountsTrie>().unwrap();
    for (k, v) in &inputs {
        account_cursor.upsert(k.clone().into(), v.clone()).unwrap();
    }
    let account_trie = DatabaseAccountTrieCursor::new(account_cursor);
    test_cursor(account_trie, &expected);

    let hashed_address = B256::random();
    let mut storage_cursor = tx.tx_ref().cursor_dup_write::<tables::StoragesTrie>().unwrap();
    for (k, v) in &inputs {
        storage_cursor
            .upsert(hashed_address, StorageTrieEntry { nibbles: k.clone().into(), node: v.clone() })
            .unwrap();
    }
    let storage_trie = DatabaseStorageTrieCursor::new(storage_cursor, hashed_address);
    test_cursor(storage_trie, &expected);
}

fn test_cursor<T>(mut trie: T, expected: &[Vec<u8>])
where
    T: TrieCursor,
{
    let mut walker = TrieWalker::new(&mut trie, Default::default());
    assert!(walker.key().unwrap().is_empty());

    // We're traversing the path in lexicographical order.
    for expected in expected {
        let got = walker.advance().unwrap();
        assert_eq!(got.unwrap(), Nibbles::from_nibbles_unchecked(expected.clone()));
    }

    // There should be 8 paths traversed in total from 3 branches.
    let got = walker.advance().unwrap();
    assert!(got.is_none());
}

#[test]
fn cursor_rootnode_with_changesets() {
    let factory = create_test_provider_factory();
    let tx = factory.provider_rw().unwrap();
    let mut cursor = tx.tx_ref().cursor_dup_write::<tables::StoragesTrie>().unwrap();

    let nodes = vec![
        (
            vec![],
            BranchNodeCompact::new(
                // 2 and 4 are set
                0b10100,
                0b00100,
                0,
                vec![],
                Some(B256::random()),
            ),
        ),
        (
            vec![0x2],
            BranchNodeCompact::new(
                // 1 is set
                0b00010,
                0,
                0b00010,
                vec![B256::random()],
                None,
            ),
        ),
    ];

    let hashed_address = B256::random();
    for (k, v) in nodes {
        cursor.upsert(hashed_address, StorageTrieEntry { nibbles: k.into(), node: v }).unwrap();
    }

    let mut trie = DatabaseStorageTrieCursor::new(cursor, hashed_address);

    // No changes
    let mut cursor = TrieWalker::new(&mut trie, Default::default());
    assert_eq!(cursor.key().cloned(), Some(Nibbles::new())); // root
    assert!(cursor.can_skip_current_node); // due to root_hash
    cursor.advance().unwrap(); // skips to the end of trie
    assert_eq!(cursor.key().cloned(), None);

    // We insert something that's not part of the existing trie/prefix.
    let mut changed = PrefixSetMut::default();
    changed.insert(Nibbles::from_nibbles([0xF, 0x1]));
    let mut cursor = TrieWalker::new(&mut trie, changed.freeze());

    // Root node
    assert_eq!(cursor.key().cloned(), Some(Nibbles::new()));
    // Should not be able to skip state due to the changed values
    assert!(!cursor.can_skip_current_node);
    cursor.advance().unwrap();
    assert_eq!(cursor.key().cloned(), Some(Nibbles::from_nibbles([0x2])));
    cursor.advance().unwrap();
    assert_eq!(cursor.key().cloned(), Some(Nibbles::from_nibbles([0x2, 0x1])));
    cursor.advance().unwrap();
    assert_eq!(cursor.key().cloned(), Some(Nibbles::from_nibbles([0x4])));

    cursor.advance().unwrap();
    assert_eq!(cursor.key().cloned(), None); // the end of trie
}

#[test]
fn test_trie_walker_with_real_db_populated_account() {
    proptest!(
        ProptestConfig::with_cases(10), | (state in arb::<State>()) | {
            let factory = create_test_provider_factory();
            let tx = factory.provider_rw().unwrap();

            for (address, (account, storage)) in &state {
                insert_account(tx.tx_ref(), *address, *account, storage)
            }
            tx.commit().unwrap();

            let tx =  factory.db_ref().tx().unwrap();

            let trie_updates = TrieUpdates::default();
            let trie_nodes_sorted = Arc::new(trie_updates.into_sorted());

            let trie_cursor_factory =
                InMemoryTrieCursorFactory::new(DatabaseTrieCursorFactory::new(&tx), &trie_nodes_sorted);

            let prefix_sets = TriePrefixSets::default();
            let mut walker = TrieWalker::new(
                trie_cursor_factory.account_trie_cursor().map_err(ProviderError::Database).unwrap(),
                prefix_sets.account_prefix_set,
    );

            assert!(walker.key().is_some());
            let _result = walker.advance().expect("Failed to advance walker");
            //assert!(result.is_some());
        }
    );
}

#[test]
fn failure() {
    let factory = create_test_provider_factory();
    let tx = factory.provider_rw().unwrap();
    let mut storage_trie_cursor = tx.tx_ref().cursor_dup_write::<tables::StoragesTrie>().unwrap();

    let hashed_address = B256::random();

    let keys = [
        &hex!("4000000000000000000000000000000000000000000000000000000000000000"),
        &hex!("4010000000000000000000000000000000000000000000000000000000000000"),
        &hex!("4200000000000000000000000000000000000000000000000000000000000000"),
        &hex!("4230000000000000000000000000000000000000000000000000000000000000"),
        &hex!("5000000000000000000000000000000000000000000000000000000000000000"),
        &hex!("5010000000000000000000000000000000000000000000000000000000000000"),
    ];

    let mut hb = HashBuilder::default().with_updates(true);
    for key in keys {
        hb.add_leaf(Nibbles::unpack(&key), &alloy_rlp::encode_fixed_size(&U256::MAX));
    }
    hb.root();
    let (_, updates) = hb.split();

    for (k, v) in updates {
        storage_trie_cursor
            .upsert(hashed_address, StorageTrieEntry { nibbles: k.into(), node: v })
            .unwrap();
    }

    let mut hashed_storage_cursor =
        tx.tx_ref().cursor_dup_write::<tables::HashedStorages>().unwrap();

    use reth_db::cursor::DbDupCursorRW;
    for key in keys {
        hashed_storage_cursor
            .append_dup(
                hashed_address,
                StorageEntry { key: B256::from_slice(key), value: U256::MAX },
            )
            .unwrap();
    }

    let mut changed = PrefixSetMut::default();
    changed.insert(Nibbles::from_nibbles([0x4, 0x2, 0x3, 0x0]));
    let mut trie_cursor = DatabaseStorageTrieCursor::new(storage_trie_cursor, hashed_address);
    let walker = TrieWalker::new(&mut trie_cursor, changed.freeze());
    let hashed_cursor = DatabaseHashedStorageCursor::new(hashed_storage_cursor, hashed_address);
    let mut node_iter = TrieNodeIter::new(walker, hashed_cursor);

    let mut hb = HashBuilder::default().with_updates(true);
    println!("\nComputing");
    while let Some(next) = node_iter.try_next().unwrap() {
        println!("next {next:?}");
        match next {
            TrieElement::Branch(branch) => {
                hb.add_branch(branch.key, branch.value, branch.children_are_in_trie);
            }
            TrieElement::Leaf(key, value) => {
                hb.add_leaf(Nibbles::unpack(key), &alloy_rlp::encode_fixed_size(&value));
            }
        }
    }
    hb.root();
    let (_, updates) = hb.split();

    let nodes = vec![
        (
            Nibbles::from_vec(vec![0x4]),
            BranchNodeCompact::new(0b101, 0b101, 0b001, vec![B256::random()], None),
        ),
        (
            Nibbles::from_vec(vec![0x4, 0x2]),
            BranchNodeCompact::new(0b1001, 0b0001, 0b0, vec![], None),
        ),
        (
            Nibbles::from_vec(vec![0x5]),
            BranchNodeCompact::new(0b1, 0b1, 0b1, vec![B256::random()], None),
        ),
    ];
    for (path, node) in &nodes {
        assert_eq!(Some(node), updates.get(path), "node mismatch at {path:?}");
        println!("node at path {path:?}: {node:?}");
    }
}
