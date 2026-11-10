//! The smoke run of the benchmarks: each runs once on the smallest group, on both prototypes,
//! natively and in WASM.

use mls_bench::{Budget, Group, Object, Operation, Results, SIZES, run};
use mls_proto_mls_rs::MlsRsDevice;
use mls_proto_openmls::OpenMlsDevice;
use mls_scenario::Device;

/// Every benchmark runs once on the smallest group and gives every object's size.
fn runs_once<D: Device>() {
    let lib = D::LIBRARY;
    let mut next = Vec::new();
    let results = run::<D>(&SIZES[..1], &Operation::ALL, Budget::ONCE, |what| {
        next.push(what.to_owned())
    })
    .unwrap();
    assert_eq!((results.library, results.sizes.as_slice()), (lib, &[2][..]));
    assert_eq!(
        results.times.iter().map(|(op, _)| *op).collect::<Vec<_>>(),
        Operation::ALL
    );
    for (operation, stats) in &results.times {
        assert_eq!(stats.len(), 1, "{lib}: {operation:?}");
        assert_eq!(stats[0].runs, 1, "{lib}: {operation:?}");
    }
    assert_eq!(
        results.objects.iter().map(|(o, _)| *o).collect::<Vec<_>>(),
        Object::ALL
    );
    for (object, sizes) in &results.objects {
        assert!(sizes.len() == 1 && sizes[0] > 0, "{lib}: {object:?}");
    }
    assert_eq!(next.len(), 1 + Operation::ALL.len(), "{lib}: {next:?}");
    assert_eq!(next[0], format!("{lib}, 2 devices: the group"));
    assert_eq!(
        next[1],
        format!("{lib}, 2 devices: Create the group with its devices")
    );
}

/// The size of `object` in `results`, one per size.
fn sizes(results: &Results, object: Object) -> &[usize] {
    &results
        .objects
        .iter()
        .find(|(o, _)| *o == object)
        .unwrap()
        .1
}

/// The objects that hold the tree or a secret for each member grow with the group; a
/// KeyPackage and a Commit with one Remove do not.
fn sizes_follow_the_group<D: Device>() {
    let lib = D::LIBRARY;
    let results = run::<D>(&[2, 10], &[], Budget::ONCE, |_| {}).unwrap();
    assert!(results.times.is_empty());
    for object in [
        Object::CreateCommit,
        Object::CreateWelcome,
        Object::AddWelcome,
        Object::GroupInfo,
        Object::State,
    ] {
        let [small, large] = sizes(&results, object) else {
            panic!("{lib}: {object:?}: a size per group");
        };
        assert!(
            large > small,
            "{lib}: {object:?}: {small} for 2, {large} for 10"
        );
    }
    let key_packages = sizes(&results, Object::KeyPackage);
    assert_eq!(key_packages[0], key_packages[1], "{lib}: KeyPackage");
    for object in [Object::AddCommit, Object::RemoveCommit, Object::EmptyCommit] {
        assert!(
            sizes(&results, object)[0] < sizes(&results, Object::AddWelcome)[1],
            "{lib}: {object:?} is smaller than a Welcome with the tree"
        );
    }
}

/// A group needs two devices: one to commit, one to process.
fn needs_two_devices<D: Device>() {
    assert!(Group::<D>::new(1).is_err(), "{}", D::LIBRARY);
    assert!(run::<D>(&[1], &[], Budget::ONCE, |_| {}).is_err());
}

macro_rules! smoke_tests {
    ($($module:ident: $device:ty),+) => {
        $(
            mod $module {
                use super::*;

                #[cfg_attr(not(target_arch = "wasm32"), test)]
                #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
                fn each_benchmark_runs_once() {
                    runs_once::<$device>();
                }

                #[cfg_attr(not(target_arch = "wasm32"), test)]
                #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
                fn object_sizes_follow_the_group() {
                    sizes_follow_the_group::<$device>();
                }

                #[cfg_attr(not(target_arch = "wasm32"), test)]
                #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
                fn a_group_needs_two_devices() {
                    needs_two_devices::<$device>();
                }
            }
        )+
    };
}

smoke_tests!(openmls: OpenMlsDevice, mls_rs: MlsRsDevice);
