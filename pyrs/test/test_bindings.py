import pytest
from pyrs import (
    RsCircuit,
    WireType,
    RsOpType,
    Subgraph,
    CircuitRewrite,
    greedy_rewrite,
    remove_redundancies,
    Direction,
)

from pytket import Circuit, OpType, Qubit


def simple_rs(op):
    c = RsCircuit()
    c.add_unitid(Qubit("q", [0]))
    i, o = c.boundary()
    v = c.add_vertex(op)
    c.add_edge((i, 0), (v, 0), WireType.Qubit)
    c.add_edge((v, 0), (o, 0), WireType.Qubit)

    return c


def test_conversion():
    c = Circuit(2).H(0).CX(0, 1)
    rc = RsCircuit.from_tket1(c)
    assert len(rc.to_tket1().get_commands()) == 2

    assert rc.dot_string()


def test_apply_rewrite():

    c = simple_rs(RsOpType.H)
    assert c.edge_endpoints(0) == ((0, 0), (2, 0))
    assert c.edge_at_port((2, 0), Direction.Outgoing) == 1
    c2 = simple_rs(RsOpType.Reset)

    c.apply_rewrite(CircuitRewrite(Subgraph({2}, [0], [1]), c2, 0.0))
    c.defrag()  # needed for exact equality check
    assert c == c2
    assert c.remove_node(2) == RsOpType.Reset
    assert c.remove_node(2) == None


@pytest.fixture()
def cx_circ() -> RsCircuit:
    return RsCircuit.from_tket1(Circuit(2).CX(0, 1).CX(0, 1))


@pytest.fixture()
def noop_circ() -> RsCircuit:
    c = Circuit(2)
    c.add_gate(OpType.noop, [0])
    c.add_gate(OpType.noop, [1])
    return RsCircuit.from_tket1(c)


def test_pattern_rewriter(cx_circ, noop_circ):
    c = Circuit(2).H(0).CX(1, 0).CX(1, 0)
    rc = RsCircuit.from_tket1(c)
    assert rc.node_edges(3, Direction.Incoming) == [1, 2]
    assert rc.neighbours(4, Direction.Outgoing) == [(1, 1), (1, 0)]

    c1 = greedy_rewrite(rc, cx_circ, lambda x: noop_circ)

    c2 = greedy_rewrite(
        rc, cx_circ, lambda x: noop_circ, lambda ni, op: op == cx_circ.node_op(ni)
    )

    correct = Circuit(2).H(0)
    correct.add_gate(OpType.noop, [1])
    correct.add_gate(OpType.noop, [0])
    for c in (c1, c2):
        assert c.to_tket1().get_commands() == correct.get_commands()


def test_equality():
    bell_circ = lambda: RsCircuit.from_tket1(Circuit(2).H(0).CX(0, 1))
    assert bell_circ() == bell_circ()
    assert bell_circ() != RsCircuit.from_tket1(Circuit(2).H(0))


def test_auto_convert():
    c = Circuit(2).CX(0, 1).CX(0, 1).Rx(2, 1)
    c2 = remove_redundancies(c)
    correct = Circuit(2).Rx(2, 1)

    assert c2 == correct
