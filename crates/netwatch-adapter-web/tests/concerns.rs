//! Which rows an event is drawn on (ADR-0031).

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015), including in helpers.

use netwatch_adapter_web::api::concerns;
use netwatch_shared::Role;

fn known() -> Vec<Role> {
    [
        "kernel",
        "nic",
        "local-interface",
        "hop:1:1",
        "hop:1:2",
        "target:1",
        "hop:2:2",
        "target:2",
    ]
    .iter()
    .map(|key| Role::from_key(key).unwrap())
    .collect()
}

#[test]
fn an_epoch_change_concerns_its_role_and_everything_downstream_of_it() {
    let hop = Role::from_key("hop:1:1");
    assert_eq!(
        concerns("epoch-started", hop, &known()),
        ["hop:1:1", "hop:1:2", "target:1"]
    );
    let everything_but_the_kernel: Vec<String> = known().iter().skip(1).map(Role::key).collect();
    assert_eq!(
        concerns("epoch-ended", Some(Role::Nic), &known()),
        everything_but_the_kernel
    );
}

#[test]
fn the_kernels_epochs_concern_only_the_kernel_because_no_path_runs_through_loopback() {
    assert_eq!(
        concerns("epoch-started", Some(Role::Kernel), &known()),
        ["kernel"]
    );
}

#[test]
fn losing_sync_concerns_the_roles_topology_binds() {
    assert_eq!(
        concerns("desynchronized", None, &known()),
        ["nic", "tunnel", "local-interface"]
    );
}

#[test]
fn a_note_concerns_only_the_role_it_names_or_else_the_kernel() {
    assert_eq!(concerns("kernel-log", Some(Role::Nic), &known()), ["nic"]);
    assert_eq!(
        concerns("clock-jump", Some(Role::Kernel), &known()),
        ["kernel"]
    );
    assert_eq!(concerns("source-disabled", None, &known()), ["kernel"]);
}

#[test]
fn a_resumed_epoch_concerns_no_role_because_nothing_changed() {
    assert!(concerns("epoch-resumed", Some(Role::Nic), &known()).is_empty());
}

#[test]
fn a_start_concerns_the_kernel_alone() {
    assert_eq!(
        concerns("started", Some(Role::Kernel), &known()),
        ["kernel"]
    );
}
