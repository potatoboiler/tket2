//! A rewriter based on circuit equivalence classes.
//!
//! Circuits are clustered in equivalence classes based on whether they
//! represent the same unitary.
//!
//! This rewriter uses the [`PatternMatcher`] to find known subcircuits and
//! generates rewrites to replace them with other circuits within the same
//! equivalence class.
//!
//! Equivalence classes are generated using Quartz
//! (<https://github.com/quantum-compiler/quartz/>). The simplest way
//! to generate such a file is to use the `gen_ecc_set.sh` script at the root
//! of the Quartz repository.

use derive_more::{From, Into};
use hugr::hugr::PortIndex;
use hugr::ops::OpTrait;
use itertools::Itertools;
use portmatching::PatternID;
use std::{
    collections::HashSet,
    fs::File,
    io,
    path::{Path, PathBuf},
};
use thiserror::Error;

use hugr::Hugr;

use crate::{
    circuit::{remove_empty_wire, Circuit},
    optimiser::taso::{load_eccs_json_file, EqCircClass},
    portmatching::{CircuitPattern, PatternMatcher},
};

use super::{CircuitRewrite, Rewriter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, From, Into, serde::Serialize, serde::Deserialize)]
struct TargetID(usize);

/// A rewriter based on circuit equivalence classes.
///
/// In every equivalence class, one circuit is chosen as the representative.
/// Valid rewrites turn a non-representative circuit into its representative,
/// or a representative circuit into any of the equivalent non-representative
/// circuits.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ECCRewriter {
    /// Matcher for finding patterns.
    matcher: PatternMatcher,
    /// Targets of some rewrite rules.
    targets: Vec<Hugr>,
    /// Rewrites, stored as a map from the source PatternID to possibly multiple
    /// target TargetIDs. The usize index of PatternID is used to index into
    /// the outer vector.
    rewrite_rules: Vec<Vec<TargetID>>,
    /// Wires that have been removed in the pattern circuit -- to be removed
    /// in the target circuit as well when generating a rewrite.
    empty_wires: Vec<Vec<usize>>,
}

impl ECCRewriter {
    /// Create a new rewriter from equivalent circuit classes in JSON file.
    ///
    /// This uses the Quartz JSON file format to store equivalent circuit classes.
    /// Generate such a file using the `gen_ecc_set.sh` script at the root of
    /// the Quartz repository.
    ///
    /// Quartz: <https://github.com/quantum-compiler/quartz/>.
    pub fn try_from_eccs_json_file(path: impl AsRef<Path>) -> io::Result<Self> {
        let eccs = load_eccs_json_file(path)?;
        Ok(Self::from_eccs(eccs))
    }

    /// Create a new rewriter from a list of equivalent circuit classes.
    ///
    /// Equivalence classes are represented as [`EqCircClass`]s, lists of
    /// HUGRs where one of the elements is chosen as the representative.
    pub fn from_eccs(eccs: impl Into<Vec<EqCircClass>>) -> Self {
        let eccs = eccs.into();
        let rewrite_rules = get_rewrite_rules(&eccs);
        let patterns = get_patterns(&eccs);
        let targets = into_targets(eccs);
        // Remove failed patterns
        let (patterns, empty_wires, rewrite_rules): (Vec<_>, Vec<_>, Vec<_>) = patterns
            .into_iter()
            .zip(rewrite_rules)
            .filter_map(|(p, r)| {
                // Filter out target IDs where empty wires are not empty
                let (pattern, pattern_empty_wires) = p?;
                let targets = r
                    .into_iter()
                    .filter(|&id| {
                        let circ = &targets[id.0];
                        let target_empty_wires: HashSet<_> =
                            empty_wires(&circ).into_iter().collect();
                        pattern_empty_wires
                            .iter()
                            .all(|&w| target_empty_wires.contains(&w))
                    })
                    .collect();
                Some((pattern, pattern_empty_wires, targets))
            })
            .multiunzip();
        let matcher = PatternMatcher::from_patterns(patterns);
        Self {
            matcher,
            targets,
            rewrite_rules,
            empty_wires,
        }
    }

    /// Get all targets of rewrite rules given a source pattern.
    fn get_targets(&self, pattern: PatternID) -> impl Iterator<Item = &Hugr> {
        self.rewrite_rules[pattern.0]
            .iter()
            .map(|id| &self.targets[id.0])
    }

    /// Serialise a rewriter to an IO stream.
    ///
    /// Precomputed rewriters can be serialised as binary and then loaded
    /// later using [`ECCRewriter::load_binary_io`].
    pub fn save_binary_io<W: io::Write>(
        &self,
        writer: &mut W,
    ) -> Result<(), RewriterSerialisationError> {
        rmp_serde::encode::write(writer, &self)?;
        Ok(())
    }

    /// Load a rewriter from an IO stream.
    ///
    /// Loads streams as created by [`ECCRewriter::save_binary_io`].
    pub fn load_binary_io<R: io::Read>(reader: &mut R) -> Result<Self, RewriterSerialisationError> {
        let matcher: Self = rmp_serde::decode::from_read(reader)?;
        Ok(matcher)
    }

    /// Save a rewriter as a binary file.
    ///
    /// Precomputed rewriters can be saved as binary files and then loaded
    /// later using [`ECCRewriter::load_binary`].
    ///
    /// The extension of the file name will always be set or amended to be
    /// `.rwr`.
    ///
    /// If successful, returns the path to the newly created file.
    pub fn save_binary(
        &self,
        name: impl AsRef<Path>,
    ) -> Result<PathBuf, RewriterSerialisationError> {
        let mut file_name = PathBuf::from(name.as_ref());
        file_name.set_extension("rwr");
        let file = File::create(&file_name)?;
        let mut file = io::BufWriter::new(file);
        self.save_binary_io(&mut file)?;
        Ok(file_name)
    }

    /// Loads a rewriter saved using [`ECCRewriter::save_binary`].
    pub fn load_binary(name: impl AsRef<Path>) -> Result<Self, RewriterSerialisationError> {
        let file = File::open(name)?;
        let mut reader = std::io::BufReader::new(file);
        Self::load_binary_io(&mut reader)
    }
}

impl Rewriter for ECCRewriter {
    fn get_rewrites<C: Circuit + Clone>(&self, circ: &C) -> Vec<CircuitRewrite> {
        let matches = self.matcher.find_matches(circ);
        matches
            .into_iter()
            .flat_map(|m| {
                let pattern_id = m.pattern_id();
                self.get_targets(pattern_id).map(move |repl| {
                    let mut repl = repl.clone();
                    for &empty_qb in self.empty_wires[pattern_id.0].iter().rev() {
                        remove_empty_wire(&mut repl, empty_qb).unwrap();
                    }
                    m.to_rewrite(circ.base_hugr(), repl)
                        .expect("invalid replacement")
                })
            })
            .collect()
    }
}

/// Errors that can occur when (de)serialising an [`ECCRewriter`].
#[derive(Debug, Error)]
pub enum RewriterSerialisationError {
    /// An IO error occured
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    /// An error occured during deserialisation
    #[error("Deserialisation error: {0}")]
    Deserialisation(#[from] rmp_serde::decode::Error),
    /// An error occured during serialisation
    #[error("Serialisation error: {0}")]
    Serialisation(#[from] rmp_serde::encode::Error),
}

fn into_targets(rep_sets: Vec<EqCircClass>) -> Vec<Hugr> {
    rep_sets
        .into_iter()
        .flat_map(|rs| rs.into_circuits())
        .collect()
}

fn get_rewrite_rules(rep_sets: &[EqCircClass]) -> Vec<Vec<TargetID>> {
    let n_circs = rep_sets.iter().map(|rs| rs.n_circuits()).sum::<usize>();
    let mut rewrite_rules = vec![Default::default(); n_circs];
    let mut curr_target = 0;
    for rep_set in rep_sets {
        let rep_ind = curr_target;
        let other_inds = (curr_target + 1)..(curr_target + rep_set.n_circuits());
        // Rewrite rules for representative circuit
        rewrite_rules[rep_ind] = other_inds.clone().map_into().collect();
        // Rewrite rules for other circuits
        for i in other_inds {
            rewrite_rules[i] = vec![rep_ind.into()];
        }
        curr_target += rep_set.n_circuits();
    }
    rewrite_rules
}

/// For an equivalence class, return all valid patterns together with the
/// indices of the wires that have been removed in the pattern circuit.
fn get_patterns(rep_sets: &[EqCircClass]) -> Vec<Option<(CircuitPattern, Vec<usize>)>> {
    rep_sets
        .iter()
        .flat_map(|rs| rs.circuits())
        .map(|circ| {
            let empty_qbs = empty_wires(circ);
            let mut circ = circ.clone();
            for &qb in empty_qbs.iter().rev() {
                remove_empty_wire(&mut circ, qb).unwrap();
            }
            CircuitPattern::try_from_circuit(&circ)
                .ok()
                .map(|circ| (circ, empty_qbs))
        })
        .collect()
}

/// The port offsets of wires that are empty.
fn empty_wires(circ: &impl Circuit) -> Vec<usize> {
    let inp = circ.input();
    circ.node_outputs(inp)
        // Only consider dataflow edges
        .filter(|&p| circ.get_optype(inp).signature().get(p).is_some())
        // Only consider ports linked to at most one other port
        .filter_map(|p| Some((p, circ.linked_ports(inp, p).at_most_one().ok()?)))
        // Ports are either connected to output or nothing
        .filter_map(|(from, to)| {
            if let Some((n, _)) = to {
                // Wires connected to output
                (n == circ.output()).then_some(from.index())
            } else {
                // Wires connected to nothing
                Some(from.index())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::{utils::build_simple_circuit, T2Op};

    use super::*;

    fn empty() -> Hugr {
        build_simple_circuit(2, |_| Ok(())).unwrap()
    }

    fn h_h() -> Hugr {
        build_simple_circuit(2, |circ| {
            circ.append(T2Op::H, [0]).unwrap();
            circ.append(T2Op::H, [0]).unwrap();
            circ.append(T2Op::CX, [0, 1]).unwrap();
            Ok(())
        })
        .unwrap()
    }

    fn cx_cx() -> Hugr {
        build_simple_circuit(2, |circ| {
            circ.append(T2Op::CX, [0, 1]).unwrap();
            circ.append(T2Op::CX, [0, 1]).unwrap();
            Ok(())
        })
        .unwrap()
    }

    fn cx_x() -> Hugr {
        build_simple_circuit(2, |circ| {
            circ.append(T2Op::CX, [0, 1]).unwrap();
            circ.append(T2Op::X, [1]).unwrap();
            Ok(())
        })
        .unwrap()
    }

    fn x_cx() -> Hugr {
        build_simple_circuit(2, |circ| {
            circ.append(T2Op::X, [1]).unwrap();
            circ.append(T2Op::CX, [0, 1]).unwrap();
            Ok(())
        })
        .unwrap()
    }

    #[test]
    fn small_ecc_rewriter() {
        let ecc1 = EqCircClass::new(h_h(), vec![empty(), cx_cx()]);
        let ecc2 = EqCircClass::new(cx_x(), vec![x_cx()]);
        let rewriter = ECCRewriter::from_eccs(vec![ecc1, ecc2]);
        assert_eq!(rewriter.targets.len(), 5);
        assert_eq!(
            rewriter.rewrite_rules,
            [
                vec![TargetID(1), TargetID(2)],
                vec![TargetID(0)],
                vec![TargetID(4)],
                vec![TargetID(3)],
            ]
        );
        assert_eq!(rewriter.get_targets(PatternID(1)).collect_vec(), [&h_h()]);
    }

    #[test]
    fn ecc_rewriter_from_file() {
        // In this example, all circuits are valid patterns, thus
        // PatternID == TargetID.
        let test_file = "test_files/small_eccs.json";
        let rewriter = ECCRewriter::try_from_eccs_json_file(test_file).unwrap();
        assert_eq!(rewriter.rewrite_rules.len(), rewriter.matcher.n_patterns());
        assert_eq!(rewriter.targets.len(), 5 * 4 + 5 * 3);

        // Assert that the rewrite rules are correct, i.e that the rewrite
        // rules in the slice (k..=k+t) is given by [[k+1, ..., k+t], [k], ..., [k]]
        // where k is the index of a representative circuit and t+1 is the size
        // of the ECC.
        let mut n_eccs_of_len = [0; 4];
        let mut next_k_are_1 = 0;
        let mut curr_repr = TargetID(0);
        for (i, rws) in rewriter.rewrite_rules.into_iter().enumerate() {
            n_eccs_of_len[rws.len()] += 1;
            if rws.len() == 1 {
                assert!(next_k_are_1 > 0);
                assert_eq!(rws, vec![curr_repr]);
                next_k_are_1 -= 1;
            } else {
                assert_eq!(next_k_are_1, 0);
                let exp_rws: Vec<_> = (i + 1..=i + rws.len()).map(TargetID).collect();
                assert_eq!(rws, exp_rws);
                next_k_are_1 = rws.len();
                curr_repr = TargetID(i);
            }
        }
        // There should be 5x ECCs of size 3 and 5x ECCs of size 4
        let exp_n_eccs_of_len = [0, 5 * 2 + 5 * 3, 5, 5];
        assert_eq!(n_eccs_of_len, exp_n_eccs_of_len);
    }

    /// Some inputs are left untouched: these parameters should be removed to
    /// obtain convex patterns
    #[test]
    fn ecc_rewriter_empty_params() {
        let test_file = "test_files/cx_cx_eccs.json";
        let rewriter = ECCRewriter::try_from_eccs_json_file(test_file).unwrap();

        let cx_cx = cx_cx();
        assert_eq!(rewriter.get_rewrites(&cx_cx).len(), 1);
    }
}
