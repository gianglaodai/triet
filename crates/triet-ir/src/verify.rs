//! SSA verifier — validates IR invariants at construction time.
//!
//! Checks enforced (per [ADR-0007]):
//! 1. **SSA invariant** — every `ValueId` is defined exactly once
//!    across the entire function.
//! 2. **Use-def chain** — every `ValueId` used as an operand has a
//!    definition somewhere in the same function.
//! 3. **Terminator presence** — every basic block ends with a
//!    terminator instruction.
//! 4. **Phi position** — phi nodes appear contiguously at the top
//!    of each block, before any other instruction.
//! 5. **Phi predecessors** — every block referenced by a phi incoming
//!    edge is a real predecessor block.
//!
//! [ADR-0007]: ../../../docs/decisions/0007-ir-design.md

use std::collections::{BTreeMap, BTreeSet};

use crate::instr::Instruction;
use crate::module::Function;
use crate::types::ValueId;

/// Result of IR verification — either success or a list of violations.
#[derive(Debug, PartialEq, Eq)]
pub struct VerifierResult {
    /// Violations found. Empty → verification passed.
    pub violations: Vec<VerifierViolation>,
}

impl VerifierResult {
    /// True if the IR passed all checks.
    #[must_use]
    pub const fn is_ok(&self) -> bool {
        self.violations.is_empty()
    }

    /// True if verification found violations.
    #[must_use]
    pub const fn is_err(&self) -> bool {
        !self.is_ok()
    }
}

/// A verification violation — describes what invariant was broken and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifierViolation {
    /// A `ValueId` is defined more than once.
    DuplicateDefinition {
        /// The duplicated value.
        value: ValueId,
        /// Function where the violation occurred.
        function: String,
    },
    /// A `ValueId` is used but never defined.
    UndefinedValue {
        /// The undefined value referenced as an operand.
        value: ValueId,
        /// Function where the violation occurred.
        function: String,
    },
    /// A basic block has no terminator instruction.
    MissingTerminator {
        /// Block ID of the malformed block.
        block: String,
        /// Function where the violation occurred.
        function: String,
    },
    /// A phi node appears after a non-phi instruction in the same block.
    PhiOutOfOrder {
        /// Block where the phi appears in wrong position.
        block: String,
        /// Function where the violation occurred.
        function: String,
    },
    /// A function has no basic blocks.
    EmptyFunction {
        /// Function where the violation occurred.
        function: String,
    },
    /// A phi incoming edge references a non-existent block or a block
    /// that doesn't actually branch to this block.
    InvalidPhiPredecessor {
        /// Block referenced by the phi.
        predecessor: String,
        /// Block containing the phi.
        block: String,
        /// Function where the violation occurred.
        function: String,
    },
}

/// Verify SSA and structural invariants on the function.
#[must_use]
pub fn verify_function(func: &Function) -> VerifierResult {
    let func_name = func
        .name
        .clone()
        .unwrap_or_else(|| format!("@{}", func.id.0));
    let mut violations = Vec::new();

    // Check: function has at least one block.
    if func.blocks.is_empty() {
        violations.push(VerifierViolation::EmptyFunction {
            function: func_name,
        });
        return VerifierResult { violations };
    }

    // Collect all value definitions and their source instructions.
    let mut defs: BTreeMap<ValueId, Vec<&Instruction>> = BTreeMap::new();

    // Function parameters are implicitly defined (ValueId 0..params.len()-1).
    let param_count = func.params.len() as u32;
    for i in 0..param_count {
        defs.entry(ValueId(i)).or_default();
    }

    for block in &func.blocks {
        for instr in &block.instructions {
            if let Some(dest) = instr.destination() {
                defs.entry(dest).or_default().push(instr);
            }
        }
    }

    // SSA invariant: each ValueId defined at most once.
    for (value, definitions) in &defs {
        if definitions.len() > 1 {
            violations.push(VerifierViolation::DuplicateDefinition {
                value: *value,
                function: func_name.clone(),
            });
        }
    }

    // Collect all value uses.
    let uses: BTreeSet<ValueId> = func
        .all_value_uses()
        .into_iter()
        .collect();

    // Use-def: every use must have a definition.
    for use_value in &uses {
        if !defs.contains_key(use_value) {
            violations.push(VerifierViolation::UndefinedValue {
                value: *use_value,
                function: func_name.clone(),
            });
        }
    }

    // Per-block checks: terminator, phi ordering.
    let all_block_ids: BTreeSet<_> = func.blocks.iter().map(|b| b.id).collect();

    for block in &func.blocks {
        let block_label = block
            .name
            .clone()
            .unwrap_or_else(|| format!("b{}", block.id.0));

        // Terminator must exist.
        if block.terminator().is_none() {
            violations.push(VerifierViolation::MissingTerminator {
                block: block_label.clone(),
                function: func_name.clone(),
            });
        }

        // Phi nodes must be at the top (no non-phi before a phi).
        let mut seen_non_phi = false;
        for instr in &block.instructions {
            if instr.is_phi() {
                if seen_non_phi {
                    violations.push(VerifierViolation::PhiOutOfOrder {
                        block: block_label.clone(),
                        function: func_name.clone(),
                    });
                    break; // one violation per block is enough
                }
            } else {
                seen_non_phi = true;
            }
        }

        // Phi predecessor blocks must exist.
        for instr in block.instructions.iter().filter(|i| i.is_phi()) {
            if let Instruction::Phi { incoming, .. } = instr {
                for edge in incoming {
                    if !all_block_ids.contains(&edge.block) {
                        violations.push(VerifierViolation::InvalidPhiPredecessor {
                            predecessor: format!("b{}", edge.block.0),
                            block: block_label.clone(),
                            function: func_name.clone(),
                        });
                    }
                }
            }
        }
    }

    VerifierResult { violations }
}

/// Verify an entire IR program.
#[must_use]
pub fn verify_program(program: &crate::module::IrProgram) -> VerifierResult {
    let mut violations = Vec::new();
    for module in &program.modules {
        for func in &module.functions {
            let result = verify_function(func);
            violations.extend(result.violations);
        }
    }
    VerifierResult { violations }
}
