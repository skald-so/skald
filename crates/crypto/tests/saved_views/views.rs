//! A view saved and loaded goes on as the view it was saved from: it has its epoch, signer and
//! members, follows each next Commit to the same view and refuses what it refuses; and a view tells
//! a GroupInfo of its own epoch's GroupContext from one of another epoch or of another group with
//! the channel's ID, a fork.

use skald_crypto::{ChannelView, MlsDevice, MlsError, Processed, SealedState};
use skald_proto::ChannelId;

use crate::fixtures::{NOW, device_id};
use crate::steps::{bytes, channel_of, step};

/// The GroupInfo device `seed` gives of its current epoch of `channel`.
fn group_info(seed: u8, state: &mut SealedState, channel: ChannelId) -> Vec<u8> {
    step(seed, state, |device| {
        bytes(device.group_info(channel).expect("a member's GroupInfo"))
    })
}

/// The Commit `make` makes on device `seed`, which then merges it as the server stored it.
fn committed(
    seed: u8,
    state: &mut SealedState,
    channel: ChannelId,
    make: impl FnOnce(&mut MlsDevice) -> Result<Vec<u8>, MlsError>,
) -> Vec<u8> {
    step(seed, state, |device| {
        let commit = make(device).expect("a Commit");
        device
            .commit_stored(channel)
            .expect("the stored Commit merges");
        commit
    })
}

/// What device `seed` does with `commit`, another device's Commit of `channel`.
fn processed(seed: u8, state: &mut SealedState, channel: ChannelId, commit: &[u8]) -> Processed {
    step(seed, state, |device| {
        (device.process_commit(channel, commit, NOW, |_| true)).expect("the device processes it")
    })
}

/// The view of `channel` from `group_info`.
fn observed(channel: ChannelId, group_info: &[u8]) -> ChannelView {
    ChannelView::observe(channel, group_info).expect("the view of a member's GroupInfo")
}

/// `view` saved, then loaded as a view of `channel`.
fn reloaded(view: &ChannelView, channel: ChannelId) -> ChannelView {
    let saved = view.save().expect("the view's bytes");
    ChannelView::load(channel, &saved).expect("a saved view loads")
}

/// `found` has the epoch, signer and members of `expected`.
#[track_caller]
fn assert_same(found: &ChannelView, expected: &ChannelView, what: &str) {
    assert_eq!(
        (found.epoch(), found.signer(), found.members()),
        (expected.epoch(), expected.signer(), expected.members()),
        "{what}: the epoch, signer and members"
    );
}

/// Both views refuse `commit`.
#[track_caller]
fn assert_both_refuse(original: &ChannelView, loaded: &ChannelView, commit: &[u8], what: &str) {
    assert!(
        original.follow(commit).is_err(),
        "the original refuses {what}"
    );
    assert!(
        loaded.follow(commit).is_err(),
        "the loaded view refuses {what}"
    );
}

/// Both views follow `commit` to the same view; gives the original's next view and the loaded
/// one's, saved and loaded again.
#[track_caller]
fn both_follow(
    original: &ChannelView,
    loaded: &ChannelView,
    commit: &[u8],
    channel: ChannelId,
    what: &str,
) -> (ChannelView, ChannelView) {
    let next = (original.follow(commit))
        .unwrap_or_else(|err| panic!("the original follows {what}: {err:?}"));
    let followed = (loaded.follow(commit))
        .unwrap_or_else(|err| panic!("the loaded view follows {what}: {err:?}"));
    assert_same(&followed, &next, what);
    let again = reloaded(&followed, channel);
    assert_same(&again, &next, &format!("{what}, saved and loaded again"));
    (next, again)
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_saved_and_loaded_view_follows_each_next_commit_as_the_original_does() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 11, &[12, 13]);
    let [first, second, third] = &mut states[..] else {
        unreachable!("three devices")
    };
    let view = observed(channel, &group_info(11, first, channel));
    let loaded = reloaded(&view, channel);
    assert_same(&loaded, &view, "loaded");

    // Devices 14 and 15 form a group of their own with the channel's ID: its Commit of epoch 1.
    let mut fork = channel_of(channel, 14, &[15]);
    let forked = step(14, &mut fork[0], |device| {
        bytes((device.commit_empty(channel, b"", NOW)).expect("an empty Commit"))
    });
    assert_both_refuse(&view, &loaded, &forked, "a fork's Commit");

    // Device 12's empty Commit.
    let empty = committed(12, second, channel, |device| {
        device.commit_empty(channel, b"", NOW)
    });
    for (seed, state) in [(11, &mut *first), (13, &mut *third)] {
        let entered = processed(seed, state, channel, &empty);
        assert_eq!(entered, Processed::Entered { epoch: 2 });
    }
    let (view, loaded) = both_follow(&view, &loaded, &empty, channel, "an empty Commit");
    assert_eq!(view.epoch(), 2);
    assert_eq!(view.signer().device, device_id(12));
    assert_both_refuse(&view, &loaded, &empty, "the Commit they followed, again");

    // Device 11 removes device 13.
    let removal = committed(11, first, channel, |device| {
        device.remove_devices(channel, &[device_id(13)], b"", NOW)
    });
    assert_eq!(
        processed(12, second, channel, &removal),
        Processed::Entered { epoch: 3 }
    );
    assert_eq!(processed(13, third, channel, &removal), Processed::Removed);
    let (view, loaded) = both_follow(&view, &loaded, &removal, channel, "a Remove");
    assert_eq!(view.epoch(), 3);
    assert_eq!(view.signer().device, device_id(11));
    let members: Vec<_> = view.members().iter().map(|member| member.device).collect();
    assert_eq!(members, [device_id(11), device_id(12)]);
    assert_both_refuse(&view, &loaded, &empty, "a Commit of an earlier epoch");

    // A view loaded from a view that was loaded goes on too.
    let last = committed(12, second, channel, |device| {
        device.commit_empty(channel, b"", NOW)
    });
    let (view, _) = both_follow(
        &view,
        &reloaded(&loaded, channel),
        &last,
        channel,
        "a fourth",
    );
    assert_eq!(view.epoch(), 4);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_view_tells_a_group_info_of_its_group_context_apart_from_others() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 21, &[22, 23]);
    let seeds = [21, 22, 23];
    let firsts: Vec<Vec<u8>> = (seeds.iter().zip(&mut states))
        .map(|(&seed, state)| group_info(seed, state, channel))
        .collect();
    let view = observed(channel, &firsts[0]);
    let loaded = reloaded(&view, channel);
    for (seed, group_info) in seeds.iter().zip(&firsts) {
        assert!(view.has_context(group_info), "device {seed}'s GroupInfo");
        assert!(loaded.has_context(group_info), "loaded: device {seed}'s");
    }

    // A fork: devices 24 and 25 form a group of their own with the channel's ID, at epoch 1 too.
    let mut fork = channel_of(channel, 24, &[25]);
    let forked = group_info(24, &mut fork[0], channel);
    assert_eq!(observed(channel, &forked).epoch(), 1, "the fork's epoch");
    assert!(!view.has_context(&forked), "a fork's GroupInfo");
    assert!(!loaded.has_context(&forked), "loaded: a fork's GroupInfo");

    // Device 22's empty Commit starts epoch 2.
    let [first, second, third] = &mut states[..] else {
        unreachable!("three devices")
    };
    let commit = committed(22, second, channel, |device| {
        device.commit_empty(channel, b"", NOW)
    });
    for (seed, state) in [(21, &mut *first), (23, &mut *third)] {
        processed(seed, state, channel, &commit);
    }
    let seconds: Vec<Vec<u8>> = (seeds.iter().zip(&mut states))
        .map(|(&seed, state)| group_info(seed, state, channel))
        .collect();
    let next = view.follow(&commit).expect("the view follows the Commit");
    for (what, follower) in [
        ("followed", &next),
        ("followed, saved and loaded", &reloaded(&next, channel)),
    ] {
        for (seed, (first, second)) in seeds.iter().zip(firsts.iter().zip(&seconds)) {
            assert!(
                follower.has_context(second),
                "{what}: device {seed}'s new GroupInfo"
            );
            assert!(
                !follower.has_context(first),
                "{what}: device {seed}'s GroupInfo of epoch 1"
            );
        }
        assert!(!follower.has_context(&forked), "{what}: a fork's GroupInfo");
    }
    // The view it followed from stays at epoch 1.
    assert!(view.has_context(&firsts[1]));
    assert!(!view.has_context(&seconds[1]));

    let message = step(21, &mut states[0], |device| {
        bytes(device.encrypt(channel, b"no GroupInfo").expect("a message"))
    });
    for (bytes, what) in [
        (Vec::new(), "no bytes"),
        (vec![0xff; 64], "bytes that are no MLS message"),
        (commit.clone(), "a Commit"),
        (message, "an application message"),
        (
            seconds[0][..seconds[0].len() - 1].to_vec(),
            "a GroupInfo cut short",
        ),
    ] {
        assert!(!next.has_context(&bytes), "{what}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn bytes_that_are_no_saved_view_of_the_channel_are_refused_without_a_panic() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 31, &[32]);
    let first = group_info(31, &mut states[0], channel);
    let saved = observed(channel, &first).save().expect("the view's bytes");

    assert!(
        ChannelView::load(ChannelId::generate(), &saved).is_err(),
        "refused: the saved view of another channel"
    );
    for (bytes, what) in [
        (Vec::new(), "no bytes"),
        (vec![0xff; 64], "bytes that are no saved view"),
        (first, "a GroupInfo"),
    ] {
        assert!(
            ChannelView::load(channel, &bytes).is_err(),
            "refused: {what}"
        );
    }
    for len in 0..saved.len() {
        assert!(
            ChannelView::load(channel, &saved[..len]).is_err(),
            "refused: the saved view cut to {len} bytes"
        );
    }
    assert_eq!(
        (ChannelView::load(channel, &saved).expect("the whole saved view")).epoch(),
        1
    );
}
