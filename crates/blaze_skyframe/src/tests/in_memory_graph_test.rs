//! Port of `InMemoryGraphTest`'s own tests (the `GraphTest` cases it inherits are in
//! `graph_test.rs`). All four are about key and label interning (`SkyKeyInterner`,
//! `usePooledInterning`, package targets), a memory optimization that is not ported.

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryGraphTest.java:99 InMemoryGraphTest.createIfAbsentBatch_skyKeyWithSkyKeyInterner
#[test]
#[ignore = "SkyKey interning (SkyKeyInterner, usePooledInterning) is not ported"]
fn create_if_absent_batch_sky_key_with_sky_key_interner() {
    unimplemented!("not ported: InMemoryGraphTest.createIfAbsentBatch_skyKeyWithSkyKeyInterner")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryGraphTest.java:122 InMemoryGraphTest.cleanupPool_weakInternerReintern
#[test]
#[ignore = "SkyKey interning (SkyKeyInterner, usePooledInterning) is not ported"]
fn cleanup_pool_weak_interner_reintern() {
    unimplemented!("not ported: InMemoryGraphTest.cleanupPool_weakInternerReintern")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryGraphTest.java:138 InMemoryGraphTest.removePackageNode_notPresentInGraph
#[test]
#[ignore = "label interning of removed package nodes (weakInternPackageTargetsLabels) is not ported"]
fn remove_package_node_not_present_in_graph() {
    unimplemented!("not ported: InMemoryGraphTest.removePackageNode_notPresentInGraph")
}

// Java: src/test/java/com/google/devtools/build/skyframe/InMemoryGraphTest.java:146 InMemoryGraphTest.removePackageNode_noValueWeakInternLabelsNoCrash
#[test]
#[ignore = "label interning of removed package nodes (weakInternPackageTargetsLabels) is not ported"]
fn remove_package_node_no_value_weak_intern_labels_no_crash() {
    unimplemented!("not ported: InMemoryGraphTest.removePackageNode_noValueWeakInternLabelsNoCrash")
}
