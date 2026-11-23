//! The modules of other test targets that the tests of tasks after their claim share, by name:
//! Ada's agents, as the tests of handing messages over have them, with the messages of the tests of
//! gap detection; what an agent reads, as the tests of compact output have it; the senders of the
//! tests of trust levels; the queue commands and requests of the tests of queues; the task states
//! and reads of the tests of task states; the claims of the tests of claiming tasks; Ada's second
//! machine of the tests of the history on new devices, with the helpers of the tests of reading the
//! history.
//!
//! A test target declares this module and imports its modules into its root, so they reach each
//! other as `crate::<module>`.

#[path = "../agent_handover/agents.rs"]
pub mod agents;
// Its states beside the claim commands, some of which a target does not name.
#[allow(unused_imports)]
#[path = "../task_claims/claiming.rs"]
pub mod claiming;
#[path = "../gap_detection/gaps.rs"]
pub mod gaps;
#[path = "../history_devices/devices.rs"]
pub mod devices;
#[path = "../compact_output/lines.rs"]
pub mod lines;
#[path = "../queues/queuing.rs"]
pub mod queuing;
#[path = "../history_reading/reading.rs"]
pub mod reading;
#[path = "../trust_levels/senders.rs"]
pub mod senders;
#[path = "../task_states/states.rs"]
pub mod states;
