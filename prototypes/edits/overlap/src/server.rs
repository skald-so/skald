//! A minimal in-process stand-in for the server: without the space key, it keeps each machine's
//! latest fingerprints and tells a machine which other machines share fingerprints with it.
//! Its only input is [`Server::publish`].

use std::collections::{BTreeMap, BTreeSet};

use crate::Fingerprint;

/// A machine as the server knows it, such as its device ID.
pub type MachineId = String;

/// What a machine sends the server: the fingerprints of its whole edit set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publication {
    pub machine: MachineId,
    pub fingerprints: BTreeSet<Fingerprint>,
}

/// Another machine that shares fingerprints with the asking one, and which.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlap {
    pub machine: MachineId,
    pub fingerprints: BTreeSet<Fingerprint>,
}

#[derive(Debug, Default)]
pub struct Server {
    latest: BTreeMap<MachineId, BTreeSet<Fingerprint>>,
    received: Vec<Publication>,
}

impl Server {
    /// Keeps the publication in place of the machine's earlier one.
    pub fn publish(&mut self, publication: Publication) {
        self.received.push(publication.clone());
        self.latest
            .insert(publication.machine, publication.fingerprints);
    }

    /// The other machines whose latest fingerprints share any with `machine`'s, in machine order.
    pub fn overlaps(&self, machine: &str) -> Vec<Overlap> {
        let Some(own) = self.latest.get(machine) else {
            return Vec::new();
        };
        self.latest
            .iter()
            .filter(|(other, _)| other.as_str() != machine)
            .filter_map(|(other, theirs)| {
                let shared: BTreeSet<_> = own.intersection(theirs).copied().collect();
                (!shared.is_empty()).then(|| Overlap {
                    machine: other.clone(),
                    fingerprints: shared,
                })
            })
            .collect()
    }

    /// Everything the server has received: each publication, in order.
    pub fn received(&self) -> &[Publication] {
        &self.received
    }

    /// Everything the server keeps: each machine's latest fingerprints.
    pub fn kept(&self) -> &BTreeMap<MachineId, BTreeSet<Fingerprint>> {
        &self.latest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn publication(machine: &str, fingerprints: &[u8]) -> Publication {
        Publication {
            machine: machine.into(),
            fingerprints: fingerprints.iter().map(|&b| Fingerprint([b; 32])).collect(),
        }
    }

    #[test]
    fn a_machine_that_has_not_published_overlaps_no_one() {
        let mut server = Server::default();
        server.publish(publication("a", &[1]));
        assert_eq!(server.overlaps("b"), []);
    }

    #[test]
    fn overlaps_list_each_other_machine_with_the_shared_fingerprints() {
        let mut server = Server::default();
        server.publish(publication("a", &[1, 2, 3]));
        server.publish(publication("b", &[2, 3, 4]));
        server.publish(publication("c", &[4, 5]));
        server.publish(publication("d", &[3]));
        assert_eq!(
            server.overlaps("a"),
            [
                Overlap {
                    machine: "b".into(),
                    fingerprints: publication("", &[2, 3]).fingerprints
                },
                Overlap {
                    machine: "d".into(),
                    fingerprints: publication("", &[3]).fingerprints
                },
            ]
        );
    }

    #[test]
    fn a_publication_replaces_the_machines_earlier_one() {
        let mut server = Server::default();
        server.publish(publication("a", &[1]));
        server.publish(publication("b", &[1]));
        server.publish(publication("a", &[2]));
        assert_eq!(server.overlaps("a"), []);
        assert_eq!(server.kept()["a"], publication("", &[2]).fingerprints);
        assert_eq!(
            server.received(),
            [
                publication("a", &[1]),
                publication("b", &[1]),
                publication("a", &[2])
            ]
        );
    }
}
