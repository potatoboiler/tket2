pub mod classical;
// pub mod redundancy;
pub mod pattern;
pub mod squash;

use crate::{
    circuit::{
        circuit::{Circuit, CircuitRewrite},
        dag::{EdgeProperties, VertexProperties},
        operation::Param,
    },
    graph::{
        graph::{DefaultIx, Direction, NodePort},
        substitute::BoundedSubgraph,
    },
};

use self::pattern::{FixedStructPattern, Match, NodeCompClosure, PatternMatcher};

/// Repeatedly apply all available rewrites reported by finder closure until no more are found.
///
/// # Errors
///
/// This function will return an error if rewrite application fails.
pub fn apply_exhaustive<F>(mut circ: Circuit, finder: F) -> Result<(Circuit, bool), String>
where
    F: Fn(&Circuit) -> Vec<CircuitRewrite>,
{
    let mut success = false;
    loop {
        // assuming all the returned rewrites are non-overlapping
        // or filter to make them non-overlapping
        // then in theory, they can all be applied in parallel
        let rewrites = finder(&circ);
        if rewrites.is_empty() {
            break;
        }
        success = true;
        for rewrite in rewrites {
            circ.apply_rewrite(rewrite)?;
        }
    }

    Ok((circ, success))
}

/// Repeatedly apply first reported rewrite
///
/// # Errors
///
/// This function will return an error if rewrite application fails.
pub fn apply_greedy<F>(mut circ: Circuit, finder: F) -> Result<(Circuit, bool), String>
where
    F: Fn(&Circuit) -> Option<CircuitRewrite>,
{
    let mut success = false;
    loop {
        if let Some(rewrite) = finder(&circ) {
            success = true;
            circ.apply_rewrite(rewrite)?;
        } else {
            break;
        }
    }

    Ok((circ, success))
}

pub type CircFixedStructPattern<F> =
    FixedStructPattern<VertexProperties, EdgeProperties, DefaultIx, F>;

impl<'p, F> CircFixedStructPattern<F> {
    pub fn from_circ(pattern_circ: Circuit, node_comp_closure: F) -> Self {
        Self {
            boundary: pattern_circ.boundary(),
            graph: pattern_circ.dag,
            node_comp_closure,
        }
    }
}
pub fn pattern_rewriter<'a, 'f: 'a, 'g: 'a, F, G>(
    pattern: CircFixedStructPattern<F>,
    circ: &'a Circuit,
    rewrite_closure: G,
) -> impl Iterator<Item = CircuitRewrite> + 'a
where
    F: NodeCompClosure<VertexProperties, EdgeProperties, DefaultIx> + Clone + 'f,
    G: Fn(Match<DefaultIx>) -> (Circuit, Param) + 'g,
{
    let matcher = PatternMatcher::new(pattern.clone(), circ.dag_ref());

    let in_ports: Vec<_> = pattern
        .graph
        .neighbours(pattern.boundary[0], Direction::Outgoing)
        .collect();
    let out_ports: Vec<_> = pattern
        .graph
        .neighbours(pattern.boundary[1], Direction::Incoming)
        .collect();
    matcher.into_matches().map(move |pmatch| {
        let in_edges: Vec<_> = in_ports
            .iter()
            .map(|np| {
                circ.dag
                    .edge_at_port(
                        NodePort::new(*pmatch.get(&np.node).unwrap(), np.port),
                        Direction::Incoming,
                    )
                    .unwrap()
            })
            .collect();
        let out_edges: Vec<_> = out_ports
            .iter()
            .map(|np| {
                circ.dag
                    .edge_at_port(
                        NodePort::new(*pmatch.get(&np.node).unwrap(), np.port),
                        Direction::Outgoing,
                    )
                    .unwrap()
            })
            .collect();
        let subg = BoundedSubgraph::new(pmatch.values().cloned().into(), [in_edges, out_edges]);

        let (newcirc, phase) = (rewrite_closure)(pmatch);

        CircuitRewrite::new(subg, newcirc.into(), phase)
    })
}

// pub struct PatternRewrites<'p, I, F> {
//     match_iter: I,
//     pattern: CircFixedStructPattern<'p, F>,
//     boundary_ports: [Vec<NodePort>; 2],
// }

// impl<'p, I, F> PatternRewrites<'p, I, F> {
//     pub fn new(
//         match_iter: I,
//         pattern: CircFixedStructPattern<'p, F>,
//         boundary_ports: [Vec<NodePort>; 2],
//     ) -> Self {
//         Self {
//             match_iter,
//             pattern,
//             boundary_ports,
//         }
//     }
// }

// impl<I: Iterator<Item = Match<DefaultIx>>, F: Fn(NodeIndex, &VertexProperties) -> bool> Iterator
//     for PatternRewrites<'_, I, F>
// {
//     type Item = CircuitRewrite;

//     fn next(&mut self) -> Option<Self::Item> {
//         self.match_iter.next().map(|pmatch| {
//             let in_edges: Vec<_> = self.boundary_ports[0]
//                 .iter()
//                 .map(|np| {
//                     self.pattern
//                         .graph
//                         .edge_at_port(
//                             NodePort::new(*pmatch.get(&np.node).unwrap(), np.port),
//                             Direction::Incoming,
//                         )
//                         .unwrap()
//                 })
//                 .collect();
//             let out_edges: Vec<_> = self.boundary_ports[1]
//                 .iter()
//                 .map(|np| {
//                     self.pattern
//                         .graph
//                         .edge_at_port(
//                             NodePort::new(*pmatch.get(&np.node).unwrap(), np.port),
//                             Direction::Outgoing,
//                         )
//                         .unwrap()
//                 })
//                 .collect();
//             let subg = BoundedSubgraph::new(pmatch.values().cloned().into(), [in_edges, out_edges]);

//             let (newcirc, phase) = (rewrite_closure)(circ, pmatch);

//             CircuitRewrite::new(subg, newcirc.into(), phase)
//         })
//     }
// }
// #[cfg(test)]
// mod tests {
//     use symengine::Expression;

//     use crate::{
//         circuit::{
//             circuit::{Circuit, UnitID},
//             operation::{Op, Param},
//         },
//         graph::graph::PortIndex,
//     };
//     use tket_json_rs::circuit_json::SerialCircuit;

//     use super::redundancy::remove_redundancies;

//     #[test]
//     fn test_remove_redundancies() {
//         // circuit with only redundant gates; identity unitary
//         //[Rz(a) q[0];, Rz(-a) q[0];, CX q[0], q[1];, CX q[0], q[1];, Rx(2) q[1];]
//         let qubits = vec![
//             UnitID::Qubit {
//                 name: "q".into(),
//                 index: vec![0],
//             },
//             UnitID::Qubit {
//                 name: "q".into(),
//                 index: vec![0],
//             },
//         ];
//         let mut circ = Circuit::with_uids(qubits);

//         circ.append_op(Op::Rz(Param::from_str("a")), &vec![PortIndex::new(0)])
//             .unwrap();
//         circ.append_op(Op::Rz(Param::new("-a")), &vec![PortIndex::new(0)])
//             .unwrap();
//         circ.append_op(Op::CX, &vec![PortIndex::new(0), PortIndex::new(1)])
//             .unwrap();
//         circ.append_op(Op::CX, &vec![PortIndex::new(0), PortIndex::new(1)])
//             .unwrap();
//         circ.append_op(Op::Rx(Param::new("2.0")), &vec![PortIndex::new(1)])
//             .unwrap();

//         let circ2 = remove_redundancies(circ);

//         let _reser: SerialCircuit<Param> = circ2.into();

//         assert_eq!(_reser.commands.len(), 0);
//         // Rx(2pi) introduces a phase
//         assert_eq!(_reser.phase, Expression::new("1.0"));
//     }
// }
