//! Whole-program CS2 type inference — the constraint propagation engine.
//!
//! A Rust port of the solver in zwyz/rs3-cache `TypePropagator.java`. The engine is
//! independent of *how* constraints are produced: callers emit type constraints
//! between [`Node`]s (locals, params, returns, vars, intermediate stack values,
//! constants) and call [`TypeInfer::propagate`] to drive every node to a fixed point
//! over the [`super::types`] lattice.
//!
//! Constraint *generation* — walking a script's instructions with a typed stack
//! simulation to emit these constraints from command signatures, gosub params,
//! enum in/out, etc. — is layered on top of this engine (G3.2b); this module is the
//! reusable core, kept separately testable.
//!
//! Presentation-only: the inferred types drive typed/named local rendering and never
//! affect encoded bytes.

use super::types::{Type, lattice};
use crate::vars::VarDomain;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;

/// The four local-variable storage classes a script allocates (mirrors zwyz's
/// `Command.LocalDomain`). Distinct from [`super::scope::LocalType`], which is the
/// coarse render-time annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocalDomain {
    Integer,
    Object,
    Long,
    Array,
}

/// A type variable in the data-flow graph. Constants and temporaries carry a unique
/// id so that two constraints to the *same* constant type are not accidentally linked
/// (matching zwyz's identity-based `ConstantType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Node {
    /// A fixed type seeded from a command signature, operand, etc. (unique per emit).
    Constant(u32),
    /// A fresh existential used by array-store constraints (unique per emit).
    Temp(u32),
    /// A script local: `(script, storage class, index)`.
    Local(i32, LocalDomain, u32),
    /// A script parameter slot: `(script, flat index)`.
    Param(i32, u32),
    /// A script return slot: `(script, index)`.
    Return(i32, u32),
    /// A player/npc/client/etc. variable.
    Var(VarDomain, u32),
    /// A varbit.
    VarBit(u32),
    /// A client variable.
    VarClient(u32),
    /// An intermediate value on the VM stack: `(expression id, slot)`.
    Expr(u32, u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ConstraintKind {
    /// `a <= b` — `a` is the same or more specific than `b` (propagated by meet,
    /// with the namedobj→obj forward-upcast exception).
    Assign,
    /// `(a <= b) or (b <= a)` — comparison; symmetric, same exception both ways.
    Compare,
    /// `a == array(b)` — `a` is the array type whose element type is `b`.
    IsArray,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Constraint {
    kind: ConstraintKind,
    a: Node,
    b: Node,
}

/// The constraint store + solved type assignment.
#[derive(Default)]
pub struct TypeInfer {
    vars: HashMap<Node, Type>,
    constraints: Vec<Constraint>,
    seen: HashSet<Constraint>,
    next_synthetic: u32,
    /// Diagnostic: the (a, b) type pairs whose meet first produced `conflict`.
    conflict_log: Vec<(Type, Type)>,
}

impl TypeInfer {
    pub fn new() -> Self {
        Self::default()
    }

    /// A fresh constant node pre-seeded with `ty`.
    pub fn constant(&mut self, ty: Type) -> Node {
        let id = self.next_synthetic;
        self.next_synthetic += 1;
        let node = Node::Constant(id);
        self.vars.insert(node, ty);
        node
    }

    fn temp(&mut self) -> Node {
        let id = self.next_synthetic;
        self.next_synthetic += 1;
        Node::Temp(id)
    }

    /// Diagnostic: the type pairs whose meet first collapsed to `conflict`.
    pub fn conflict_log(&self) -> &[(Type, Type)] {
        &self.conflict_log
    }

    /// The current type of a node (`unknown` if never constrained).
    pub fn type_of(&self, node: Node) -> Type {
        self.vars
            .get(&node)
            .copied()
            .unwrap_or_else(|| lattice().wk().unknown)
    }

    fn push(&mut self, constraint: Constraint) {
        if self.seen.insert(constraint) {
            self.constraints.push(constraint);
        }
    }

    /// `a <= b`.
    pub fn assign(&mut self, a: Node, b: Node) {
        self.push(Constraint {
            kind: ConstraintKind::Assign,
            a,
            b,
        });
    }

    /// `a <= b` to a fixed type.
    pub fn assign_type(&mut self, a: Node, ty: Type) {
        let b = self.constant(ty);
        self.assign(a, b);
    }

    /// `a == b` (both directions).
    pub fn equal(&mut self, a: Node, b: Node) {
        self.assign(a, b);
        self.assign(b, a);
    }

    /// `a == b` to a fixed type.
    pub fn equal_type(&mut self, a: Node, ty: Type) {
        let b = self.constant(ty);
        self.equal(a, b);
    }

    /// Symmetric comparison constraint.
    pub fn compare(&mut self, a: Node, b: Node) {
        self.push(Constraint {
            kind: ConstraintKind::Compare,
            a,
            b,
        });
    }

    /// `a` is an array whose element type is `b`.
    pub fn is_array(&mut self, a: Node, b: Node) {
        self.push(Constraint {
            kind: ConstraintKind::IsArray,
            a,
            b,
        });
    }

    /// Store into an array: `exists t. value <= t  and  isarray(array, t)`.
    pub fn array_store(&mut self, value: Node, array: Node) {
        let t = self.temp();
        self.assign(value, t);
        self.is_array(array, t);
    }

    /// Drive every node to a fixed point. Mirrors `TypePropagator.propagateUntilStable`:
    /// a worklist over constraints, re-queuing a constraint's neighbours whenever a
    /// node's type changes, until nothing changes.
    pub fn propagate(&mut self) {
        let lat = lattice();
        let wk = lat.wk();

        // node -> indices of incident constraints
        let mut incident: HashMap<Node, Vec<usize>> = HashMap::new();
        for (i, c) in self.constraints.iter().enumerate() {
            incident.entry(c.a).or_default().push(i);
            incident.entry(c.b).or_default().push(i);
        }

        let mut queue: VecDeque<usize> = (0..self.constraints.len()).collect();
        let mut queued: HashSet<usize> = (0..self.constraints.len()).collect();

        while let Some(idx) = queue.pop_front() {
            queued.remove(&idx);
            let Constraint { kind, a, b } = self.constraints[idx];
            let prev_a = self.type_of(a);
            let prev_b = self.type_of(b);
            let mut type_a = prev_a;
            let mut type_b = prev_b;

            match kind {
                ConstraintKind::Assign | ConstraintKind::Compare => {
                    if type_a == type_b {
                        // nothing to do
                    } else if type_a == wk.namedobj && lat.test(type_a, type_b) {
                        // namedobj only flows forward as obj (allow upcasting)
                        type_b = wk.obj;
                    } else if type_b == wk.namedobj
                        && lat.test(type_b, type_a)
                        && kind == ConstraintKind::Compare
                    {
                        type_a = wk.obj;
                    } else {
                        let met = lat.meet(type_a, type_b);
                        if met == wk.conflict && type_a != wk.conflict && type_b != wk.conflict {
                            self.conflict_log.push((type_a, type_b));
                        }
                        type_a = met;
                        type_b = met;
                    }
                }
                ConstraintKind::IsArray => {
                    let element_a = type_a.element().unwrap_or(wk.unknown);
                    let met = lat.meet(element_a, type_b);
                    let met_array = met.array().unwrap_or(met);
                    type_a = lat.meet(type_a, met_array);
                    type_b = lat.meet(type_b, met);
                }
            }

            if prev_a != type_a {
                self.vars.insert(a, type_a);
                requeue(&incident, a, &mut queue, &mut queued);
            }
            if prev_b != type_b {
                self.vars.insert(b, type_b);
                requeue(&incident, b, &mut queue, &mut queued);
            }
        }
    }
}

fn requeue(
    incident: &HashMap<Node, Vec<usize>>,
    node: Node,
    queue: &mut VecDeque<usize>,
    queued: &mut HashSet<usize>,
) {
    if let Some(indices) = incident.get(&node) {
        for &i in indices {
            if queued.insert(i) {
                queue.push_back(i);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transpile::types::lattice;

    fn ty(name: &str) -> Type {
        lattice().by_name(name)
    }

    #[test]
    fn equal_locals_share_inferred_type() {
        let mut inf = TypeInfer::new();
        let l0 = Node::Local(1, LocalDomain::Integer, 0);
        let l1 = Node::Local(1, LocalDomain::Integer, 1);
        inf.assign_type(l0, ty("component"));
        inf.equal(l0, l1);
        inf.propagate();
        assert_eq!(inf.type_of(l0).name(), "component");
        assert_eq!(inf.type_of(l1).name(), "component");
    }

    #[test]
    fn specific_type_refines_unknown_int() {
        // a local seeded generic-int then constrained loc should land on loc.
        let mut inf = TypeInfer::new();
        let l0 = Node::Local(1, LocalDomain::Integer, 0);
        inf.assign_type(l0, lattice().wk().unknown_int);
        inf.assign_type(l0, ty("loc"));
        inf.propagate();
        assert_eq!(inf.type_of(l0).name(), "loc");
    }

    #[test]
    fn conflicting_aliases_collapse_to_int_int() {
        let mut inf = TypeInfer::new();
        let l0 = Node::Local(1, LocalDomain::Integer, 0);
        inf.assign_type(l0, ty("intbool"));
        inf.assign_type(l0, ty("key"));
        inf.propagate();
        assert_eq!(inf.type_of(l0), lattice().wk().int_int);
    }

    #[test]
    fn namedobj_upcasts_forward_to_obj() {
        // namedobj flowing into an unconstrained slot upcasts to obj.
        let mut inf = TypeInfer::new();
        let l0 = Node::Local(1, LocalDomain::Object, 0);
        let src = inf.constant(lattice().wk().namedobj);
        inf.assign(src, l0);
        inf.propagate();
        assert_eq!(inf.type_of(l0), lattice().wk().obj);
    }

    #[test]
    fn type_flows_through_gosub_param_and_return() {
        // caller arg (component) -> callee param; callee return (loc) -> caller result.
        let mut inf = TypeInfer::new();
        let arg = Node::Expr(0, 0);
        let callee_param = Node::Param(42, 0);
        inf.assign_type(arg, ty("component"));
        inf.assign(arg, callee_param);

        let callee_ret = Node::Return(42, 0);
        let caller_result = Node::Expr(1, 0);
        inf.assign_type(callee_ret, ty("loc"));
        inf.equal(caller_result, callee_ret);

        inf.propagate();
        assert_eq!(inf.type_of(callee_param).name(), "component");
        assert_eq!(inf.type_of(caller_result).name(), "loc");
    }

    #[test]
    fn array_store_infers_element_type() {
        // storing a `component` into array local 0 makes it a componentarray,
        // and reading the element back yields component.
        let mut inf = TypeInfer::new();
        let array = Node::Local(1, LocalDomain::Array, 0);
        let value = inf.constant(ty("component"));
        inf.array_store(value, array);
        inf.propagate();
        assert_eq!(inf.type_of(array).name(), "componentarray");
    }

    #[test]
    fn unconstrained_node_is_unknown() {
        let inf = TypeInfer::new();
        assert_eq!(
            inf.type_of(Node::Local(1, LocalDomain::Integer, 0)),
            lattice().wk().unknown
        );
    }
}
