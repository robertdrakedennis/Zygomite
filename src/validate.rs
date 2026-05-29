use crate::dep_tree::ResolverContext;
use crate::script::{CompiledScript, OpcodeBook, Operand};
use crate::transpile::{
    ScriptCatalog, ScriptCatalogBuilder, ScriptId, ScriptSignature, build_script_catalog,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Error types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ValidationError {
    UnknownOpcode {
        index: usize,
        opcode: u16,
    },
    InvalidBranchTarget {
        index: usize,
        target: i32,
        total_instructions: usize,
    },
    VarpNotFound {
        index: usize,
        domain: String,
        id: u32,
    },
    VarbitNotFound {
        index: usize,
        id: u32,
    },
    ScriptNotFound {
        index: usize,
        called_id: i32,
    },
    /// Popping from a typed stack that has insufficient values.
    StackUnderflow {
        index: usize,
        stack: String,
        needed: usize,
        available: usize,
    },
    UnbalancedReturn {
        index: usize,
        int_stack: usize,
        obj_stack: usize,
        long_stack: usize,
    },
    MissingReturn,
}

#[derive(Debug, Clone, Serialize)]
pub struct ValidationReport {
    pub script_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_name: Option<String>,
    pub build: u32,
    pub instruction_count: usize,
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<String>,
}

impl ValidationReport {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

// ── Three typed stacks (matching the Ignis runtime) ──

struct TypedStacks {
    ints: Vec<()>,
    objects: Vec<()>,
    longs: Vec<()>,
    unknowns: Vec<()>,
}

impl TypedStacks {
    fn new() -> Self {
        Self {
            ints: Vec::new(),
            objects: Vec::new(),
            longs: Vec::new(),
            unknowns: Vec::new(),
        }
    }
}

/// Per-stack pop/push counts resulting from an instruction.
struct StackEffect {
    pops_int: usize,
    pops_obj: usize,
    pops_long: usize,
    pushes_int: usize,
    pushes_obj: usize,
    pushes_long: usize,
    pushes_unknown: usize,
}

impl StackEffect {
    const fn int_push(n: usize) -> Self {
        Self {
            pushes_int: n,
            pushes_obj: 0,
            pushes_long: 0,
            pushes_unknown: 0,
            pops_int: 0,
            pops_obj: 0,
            pops_long: 0,
        }
    }
    const fn obj_push(n: usize) -> Self {
        Self {
            pushes_obj: n,
            pushes_int: 0,
            pushes_long: 0,
            pushes_unknown: 0,
            pops_int: 0,
            pops_obj: 0,
            pops_long: 0,
        }
    }
    const fn long_push(n: usize) -> Self {
        Self {
            pushes_long: n,
            pushes_int: 0,
            pushes_obj: 0,
            pushes_unknown: 0,
            pops_int: 0,
            pops_obj: 0,
            pops_long: 0,
        }
    }
    const fn int_pop(n: usize) -> Self {
        Self {
            pops_int: n,
            pops_obj: 0,
            pops_long: 0,
            pushes_int: 0,
            pushes_obj: 0,
            pushes_long: 0,
            pushes_unknown: 0,
        }
    }
    const fn int_op(n: usize) -> Self {
        Self {
            pops_int: n,
            pushes_int: 1,
            pops_obj: 0,
            pops_long: 0,
            pushes_obj: 0,
            pushes_long: 0,
            pushes_unknown: 0,
        }
    }
    const fn obj_pop(n: usize) -> Self {
        Self {
            pops_obj: n,
            pops_int: 0,
            pops_long: 0,
            pushes_int: 0,
            pushes_obj: 0,
            pushes_long: 0,
            pushes_unknown: 0,
        }
    }
    const fn long_pop(n: usize) -> Self {
        Self {
            pops_long: n,
            pops_int: 0,
            pops_obj: 0,
            pushes_int: 0,
            pushes_obj: 0,
            pushes_long: 0,
            pushes_unknown: 0,
        }
    }
    const fn none() -> Self {
        Self {
            pops_int: 0,
            pops_obj: 0,
            pops_long: 0,
            pushes_int: 0,
            pushes_obj: 0,
            pushes_long: 0,
            pushes_unknown: 0,
        }
    }
}

// ── Validator ──

pub struct Cs2Validator<'a> {
    ctx: &'a ResolverContext,
}

impl<'a> Cs2Validator<'a> {
    pub fn new(ctx: &'a ResolverContext) -> Self {
        Self { ctx }
    }

    pub fn validate(&self, script_id: u32) -> ValidationReport {
        let Some(script) = self.ctx.decoded_scripts.get(&script_id) else {
            return missing_script_report(script_id, self.ctx.build);
        };
        let script_catalog = build_validation_catalog(self.ctx, &[]);
        let script_signatures = script_catalog.signature_map();

        self.validate_compiled(
            script_id,
            script,
            &script_catalog,
            &script_signatures,
            script.name.clone(),
        )
    }

    pub fn validate_compiled(
        &self,
        script_id: u32,
        script: &CompiledScript,
        script_catalog: &ScriptCatalog,
        script_signatures: &HashMap<ScriptId, ScriptSignature>,
        script_name: Option<String>,
    ) -> ValidationReport {
        let mut report = ValidationReport {
            script_id,
            script_name,
            build: self.ctx.build,
            instruction_count: script.code.len(),
            errors: Vec::new(),
            warnings: Vec::new(),
        };

        self.pass_structural(script, &mut report);
        self.pass_stack(script, script_catalog, script_signatures, &mut report);
        self.pass_cross_ref(script, script_catalog, &mut report);
        report
    }

    fn pass_structural(&self, script: &CompiledScript, report: &mut ValidationReport) {
        let total = script.code.len();
        for (i, instr) in script.code.iter().enumerate() {
            if self.ctx.opcode_book.name(instr.opcode).is_err()
                && !instr.command.starts_with("cmd_")
            {
                report.errors.push(ValidationError::UnknownOpcode {
                    index: i,
                    opcode: instr.opcode,
                });
            }

            match &instr.operand {
                Operand::Branch(offset) if *offset < 0 || *offset as usize >= total => {
                    report.errors.push(ValidationError::InvalidBranchTarget {
                        index: i,
                        target: *offset,
                        total_instructions: total,
                    });
                }
                Operand::Switch(cases) => {
                    for case in cases {
                        if case.target < 0 || case.target as usize >= total {
                            report.errors.push(ValidationError::InvalidBranchTarget {
                                index: i,
                                target: case.target,
                                total_instructions: total,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn pass_stack(
        &self,
        script: &CompiledScript,
        script_catalog: &crate::transpile::ScriptCatalog,
        script_signatures: &std::collections::HashMap<
            crate::transpile::ScriptId,
            crate::transpile::ScriptSignature,
        >,
        report: &mut ValidationReport,
    ) {
        let mut stacks = TypedStacks::new();

        for (i, instr) in script.code.iter().enumerate() {
            let effect = self.stack_effect_for(instr, script_catalog, script_signatures);

            Self::apply_int_pop(&mut stacks, effect.pops_int, i, report);
            Self::apply_obj_pop(&mut stacks, effect.pops_obj, i, report);
            Self::apply_long_pop(&mut stacks, effect.pops_long, i, report);

            for _ in 0..effect.pushes_int {
                stacks.ints.push(());
            }
            for _ in 0..effect.pushes_obj {
                stacks.objects.push(());
            }
            for _ in 0..effect.pushes_long {
                stacks.longs.push(());
            }
            for _ in 0..effect.pushes_unknown {
                stacks.unknowns.push(());
            }
        }

        match script.code.last().map(|i| i.command.as_str()) {
            Some("return" | "branch") => {}
            Some(cmd) => report
                .warnings
                .push(format!("ends with '{cmd}' (expected 'return' or 'branch')")),
            None => report.errors.push(ValidationError::MissingReturn),
        }
    }

    fn apply_int_pop(
        stacks: &mut TypedStacks,
        needed: usize,
        index: usize,
        report: &mut ValidationReport,
    ) {
        let available = stacks.ints.len() + stacks.unknowns.len();
        if available < needed {
            report.errors.push(ValidationError::StackUnderflow {
                index,
                stack: "int".to_string(),
                needed,
                available,
            });
            stacks.ints.clear();
            stacks.unknowns.clear();
        } else {
            let typed_used = needed.min(stacks.ints.len());
            stacks.ints.truncate(stacks.ints.len() - typed_used);
            let unknown_used = needed - typed_used;
            if unknown_used > 0 {
                stacks
                    .unknowns
                    .truncate(stacks.unknowns.len() - unknown_used);
            }
        }
    }

    fn apply_obj_pop(
        stacks: &mut TypedStacks,
        needed: usize,
        index: usize,
        report: &mut ValidationReport,
    ) {
        let available = stacks.objects.len() + stacks.unknowns.len();
        if available < needed {
            report.errors.push(ValidationError::StackUnderflow {
                index,
                stack: "obj".to_string(),
                needed,
                available,
            });
            stacks.objects.clear();
            stacks.unknowns.clear();
        } else {
            let typed_used = needed.min(stacks.objects.len());
            stacks.objects.truncate(stacks.objects.len() - typed_used);
            let unknown_used = needed - typed_used;
            if unknown_used > 0 {
                stacks
                    .unknowns
                    .truncate(stacks.unknowns.len() - unknown_used);
            }
        }
    }

    fn apply_long_pop(
        stacks: &mut TypedStacks,
        needed: usize,
        index: usize,
        report: &mut ValidationReport,
    ) {
        let available = stacks.longs.len() + stacks.unknowns.len();
        if available < needed {
            report.errors.push(ValidationError::StackUnderflow {
                index,
                stack: "long".to_string(),
                needed,
                available,
            });
            stacks.longs.clear();
            stacks.unknowns.clear();
        } else {
            let typed_used = needed.min(stacks.longs.len());
            stacks.longs.truncate(stacks.longs.len() - typed_used);
            let unknown_used = needed - typed_used;
            if unknown_used > 0 {
                stacks
                    .unknowns
                    .truncate(stacks.unknowns.len() - unknown_used);
            }
        }
    }

    fn pass_cross_ref(
        &self,
        script: &CompiledScript,
        script_catalog: &crate::transpile::ScriptCatalog,
        report: &mut ValidationReport,
    ) {
        for (i, instr) in script.code.iter().enumerate() {
            match &instr.operand {
                Operand::VarRef(vr) => {
                    let exists = self
                        .ctx
                        .varps_by_domain
                        .get(&vr.domain)
                        .and_then(|vars| vars.get(&u32::from(vr.id)))
                        .is_some();
                    if !exists {
                        report.errors.push(ValidationError::VarpNotFound {
                            index: i,
                            domain: vr.domain.as_label().to_string(),
                            id: u32::from(vr.id),
                        });
                    }
                }
                Operand::VarBitRef(vbr) => {
                    if !self.ctx.varbits.contains_key(&u32::from(vbr.id)) {
                        report.errors.push(ValidationError::VarbitNotFound {
                            index: i,
                            id: u32::from(vbr.id),
                        });
                    }
                }
                Operand::Script(called_id) => {
                    if script_catalog.resolve_call_target(*called_id).is_none() {
                        report.warnings.push(format!(
                            "[{i}] gosub_with_params to script {called_id}: not found in build {}",
                            self.ctx.build
                        ));
                    }
                }
                _ => {}
            }
        }
    }

    // Based on Ignis ClientScriptState runtime: three typed stacks
    // (intStack/isp, objectStack/osp, longStack/lsp).
    fn stack_effect_for(
        &self,
        instr: &crate::script::Instruction,
        script_catalog: &crate::transpile::ScriptCatalog,
        script_signatures: &std::collections::HashMap<
            crate::transpile::ScriptId,
            crate::transpile::ScriptSignature,
        >,
    ) -> StackEffect {
        let cmd = &instr.command;
        match cmd.as_str() {
            // ── Integer pushes ──
            "push_constant_int" => StackEffect::int_push(1),
            "push_int_local" => StackEffect::int_push(1),

            // ── Object (string) pushes ──
            // push_constant_string is a typed-constant opcode (v>=800): a tag
            // byte selects int/long/string, decoded into the operand variant.
            // Resolve the stack effect from that variant rather than assuming a
            // string push, or an int-tagged constant followed by int ops would
            // falsely underflow.
            "push_constant_string" => match &instr.operand {
                Operand::Int(_) => StackEffect::int_push(1),
                Operand::Long(_) => StackEffect::long_push(1),
                _ => StackEffect::obj_push(1),
            },
            "push_string_local" => StackEffect::obj_push(1),

            // ── Long pushes ──
            "push_long_constant" | "push_constant_long" => StackEffect::long_push(1),
            "push_long_local" => StackEffect::long_push(1),

            // ── push_var: resolve varp type to determine int/obj/long ──
            "push_var" => self.varp_stack_effect_for(&instr.operand, true),
            "push_varc_int"
            | "push_varc_string"
            | "push_varclan"
            | "push_varclan_long"
            | "push_varclan_string"
            | "push_varclansetting"
            | "push_varclansetting_long"
            | "push_varclansetting_string" => self.varp_stack_effect_for(&instr.operand, true),
            "push_varclanbit" | "push_varclansettingbit" => StackEffect::int_push(1),

            // ── Integer pops ──
            "pop_int_local" => StackEffect::int_pop(1),
            "pop_int_discard" => StackEffect::int_pop(1),

            // ── Object pops ──
            "pop_string_local" => StackEffect::obj_pop(1),
            "pop_string_discard" => StackEffect::obj_pop(1),

            // ── Long pops ──
            "pop_long_local" | "pop_long_discard" => StackEffect::long_pop(1),

            // ── pop_var: resolve varp type ──
            "pop_var" => self.varp_stack_effect_for(&instr.operand, false),
            "pop_varc_int" | "pop_varc_string" => self.varp_stack_effect_for(&instr.operand, false),

            // ── Integer arithmetic: pop 2, push 1 (opcode is `modulo`, not `mod`) ──
            "add" | "sub" | "multiply" | "divide" | "modulo" => StackEffect::int_op(2),

            // ── Logical: pop 2 ints, push 1 int ──
            "and" | "or" | "compare" => StackEffect::int_op(2),

            // ── String ops: pop 1 obj, push 1 ──
            "lowercase" | "uppercase" => StackEffect {
                pops_obj: 1,
                pushes_obj: 1,
                ..StackEffect::none()
            },
            "length" => StackEffect {
                pops_obj: 1,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "join_string" => match &instr.operand {
                Operand::Count(count) => StackEffect {
                    pops_obj: usize::try_from(*count).unwrap_or(0),
                    pushes_obj: 1,
                    ..StackEffect::none()
                },
                _ => StackEffect {
                    pops_obj: 0,
                    pushes_obj: 1,
                    ..StackEffect::none()
                },
            },

            // ── Unary int: pop 1, push 1 ──
            "neg" => StackEffect::int_op(1),

            // ── Control flow: no stack effect ──
            "branch" => StackEffect::none(),
            "branch_not" | "branch_if_true" | "branch_if_false" => StackEffect::int_pop(1),
            "branch_equals"
            | "branch_less_than"
            | "branch_greater_than"
            | "branch_less_than_or_equals"
            | "branch_greater_than_or_equals" => StackEffect {
                pops_int: 2,
                ..StackEffect::none()
            },
            "long_branch_equals"
            | "long_branch_less_than"
            | "long_branch_greater_than"
            | "long_branch_less_than_or_equals"
            | "long_branch_greater_than_or_equals" => StackEffect {
                pops_long: 2,
                ..StackEffect::none()
            },
            "switch" => StackEffect::int_pop(1),
            "return" => StackEffect::none(),
            "gosub_with_params" => {
                self.gosub_stack_effect(&instr.operand, script_catalog, script_signatures)
            }

            // ── Array ops ──
            "define_array" => StackEffect::none(),
            "push_array_int" => StackEffect {
                pops_int: 1,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "push_array_int_leave_index_on_stack" | "push_array_int_and_index" => StackEffect {
                pops_int: 1,
                pushes_int: 2,
                ..StackEffect::none()
            },
            "pop_array_int" => StackEffect {
                pops_int: 2,
                ..StackEffect::none()
            },
            "pop_array_int_leave_value_on_stack" => StackEffect {
                pops_int: 2,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "push_array_string" => StackEffect {
                pops_obj: 1,
                pushes_obj: 1,
                ..StackEffect::none()
            },
            "pop_array_string" => StackEffect {
                pops_obj: 2,
                ..StackEffect::none()
            },

            // ── Varbit: pops int for bit index, pushes int value ──
            "push_varbit" => StackEffect::int_push(1),
            "pop_varbit" => StackEffect::int_pop(1),
            "player_group_find"
            | "player_group_member_count"
            | "player_group_banned_count"
            | "player_group_get_max_size"
            | "player_group_get_create_mins_since_epoch"
            | "player_group_get_create_seconds_to_now"
            | "player_group_is_members_only"
            | "player_group_get_overall_status"
            | "player_group_get_owner_slot"
            | "activeclanchannel_find_affined"
            | "activeclanchannel_find_listened"
            | "activeclanchannel_getrankkick"
            | "activeclanchannel_getranktalk"
            | "activeclanchannel_getusercount"
            | "activeclansettings_find_affined"
            | "activeclansettings_find_listened"
            | "activeclansettings_getaffinedcount"
            | "activeclansettings_getallowunaffined"
            | "activeclansettings_getbannedcount"
            | "activeclansettings_getcoinshare"
            | "activeclansettings_getcurrentowner_slot"
            | "activeclansettings_getrankkick"
            | "activeclansettings_getranklootshare"
            | "activeclansettings_getranktalk"
            | "activeclansettings_getreplacementowner_slot"
            | "clanprofile_find" => StackEffect::int_push(1),
            "player_group_member_get_rank"
            | "player_group_member_get_team"
            | "player_group_member_get_last_seen_node_id"
            | "player_group_member_get_status"
            | "player_group_member_is_online"
            | "player_group_member_is_member"
            | "player_group_member_is_owner"
            | "activeclanchannel_getuserrank"
            | "activeclanchannel_getuserworld"
            | "activeclanchannel_getsorteduserslot"
            | "activeclansettings_getaffinedrank"
            | "activeclansettings_getaffinedmuted"
            | "activeclansettings_getaffinedjoinruneday"
            | "activeclansettings_getsortedaffinedslot" => StackEffect {
                pops_int: 1,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "player_group_member_get_displayname"
            | "player_group_banned_get_displayname"
            | "activeclanchannel_getuserdisplayname"
            | "activeclansettings_getaffineddisplayname"
            | "activeclansettings_getbanneddisplayname" => StackEffect {
                pops_int: 1,
                pushes_obj: 1,
                ..StackEffect::none()
            },
            "player_group_get_displayname"
            | "activeclanchannel_getclanname"
            | "activeclansettings_getclanname" => StackEffect::obj_push(1),
            "player_group_member_get_join_xp" | "activeclansettings_getaffinedextrainfo" => {
                StackEffect {
                    pops_int: 3,
                    pushes_int: 1,
                    ..StackEffect::none()
                }
            }
            "player_group_member_get_same_world_var" => StackEffect {
                pops_int: 3,
                pushes_unknown: 1,
                ..StackEffect::none()
            },
            "activeclanchannel_getuserslot" | "activeclansettings_getaffinedslot" => StackEffect {
                pops_obj: 1,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "activeclanchannel_kickuser" | "affinedclansettings_addbanned_fromchannel" => {
                StackEffect::int_pop(1)
            }
            "affinedclansettings_setmuted_fromchannel" => StackEffect {
                pops_int: 2,
                ..StackEffect::none()
            },
            "login_last_transfer_reply" => StackEffect {
                pushes_int: 3,
                ..StackEffect::none()
            },
            "login_inprogress"
            | "login_queue_position"
            | "login_disallowtrigger"
            | "create_reply"
            | "create_email_validate_reply"
            | "create_name_validate_reply"
            | "create_connect_reply"
            | "shop_requestdatastatus"
            | "shop_getcategorycount"
            | "autosetup_getlevel"
            | "create_under13" => StackEffect::int_push(1),
            "create_suggest_name_reply" => StackEffect {
                pushes_int: 1,
                pushes_obj: 1,
                ..StackEffect::none()
            },
            "create_get_email" => StackEffect::obj_push(1),
            "sso_displayname" => StackEffect::obj_push(1),
            "login_request_social_network" => StackEffect {
                pops_int: 2,
                pops_obj: 1,
                ..StackEffect::none()
            },
            "lobby_entergame" => StackEffect {
                pops_int: 1,
                pops_obj: 1,
                ..StackEffect::none()
            },
            "lobby_enterlobby" => StackEffect {
                pops_int: 1,
                pops_obj: 3,
                ..StackEffect::none()
            },
            "lobby_enterlobby_sso" => StackEffect {
                pops_int: 1,
                pops_obj: 1,
                ..StackEffect::none()
            },
            "create_availablerequest" | "create_name_availablerequest" | "login_accountappeal" => {
                StackEffect {
                    pops_obj: 1,
                    pushes_int: usize::from(cmd == "login_accountappeal"),
                    ..StackEffect::none()
                }
            }
            "create_createrequest" => StackEffect {
                pops_int: 2,
                pops_obj: 3,
                ..StackEffect::none()
            },
            "create_suggest_name_request"
            | "create_connectrequest"
            | "login_resetreply"
            | "login_cancel"
            | "login_continue"
            | "shop_requestdata"
            | "shop_applypendingtransactions"
            | "marketing_init"
            | "notifications_init"
            | "notifications_opensettings"
            | "autosetup_setultra"
            | "autosetup_sethigh"
            | "autosetup_setmedium"
            | "autosetup_setlow"
            | "autosetup_setmin"
            | "autosetup_setcustom"
            | "autosetup_blackflaglast"
            | "fullscreen_exit"
            | "create_setunder13" => StackEffect::none(),
            "create_step_reached" | "shop_open" | "shop_getcategoryid" | "shop_getproductcount" => {
                StackEffect {
                    pops_int: 1,
                    pushes_int: usize::from(
                        cmd == "shop_getcategoryid" || cmd == "shop_getproductcount",
                    ),
                    ..StackEffect::none()
                }
            }
            "lobby_entergamereply"
            | "lobby_enterlobbyreply"
            | "shop_purchaseitemstatus"
            | "sso_available" => StackEffect::int_push(1),
            "shop_getindexforcategoryid" => StackEffect {
                pops_int: 1,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "shop_getindexforcategoryname" => StackEffect {
                pops_obj: 1,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "shop_getcategorydescription" => StackEffect {
                pops_int: 1,
                pushes_obj: 1,
                ..StackEffect::none()
            },
            "shop_isproductavailable" | "shop_isproductrecommended" => StackEffect {
                pops_int: 2,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "shop_purchaseitem" | "marketing_sendevent" => StackEffect {
                pops_obj: 1,
                ..StackEffect::none()
            },
            "shop_getproductdetails" => StackEffect {
                pops_int: 2,
                pushes_obj: 9,
                ..StackEffect::none()
            },
            "get_currentcursor" | "get_mousex" | "get_mousey" => StackEffect::int_push(1),
            "get_mousebuttons" | "get_minimenu_length" => StackEffect {
                pushes_int: if cmd == "get_mousebuttons" { 3 } else { 2 },
                ..StackEffect::none()
            },
            "worldlist_start" | "worldlist_next" => StackEffect {
                pushes_int: 5,
                pushes_obj: 3,
                ..StackEffect::none()
            },
            "worldlist_fetch" | "worldlist_specific_thisworld" => StackEffect::int_push(1),
            "worldlist_specific" => StackEffect {
                pops_int: 1,
                pushes_int: 4,
                pushes_obj: 3,
                ..StackEffect::none()
            },
            "worldlist_switch" => StackEffect {
                pops_int: 1,
                pops_obj: 1,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "worldlist_sort" => StackEffect {
                pops_int: 4,
                ..StackEffect::none()
            },
            "worldlist_pingworlds" => StackEffect::int_pop(1),
            "worldlist_autoworld" => StackEffect::none(),
            "pushCanvasSize"
            | "viewport_geteffectivesize"
            | "viewport_getzoom"
            | "viewport_getfov" => StackEffect {
                pushes_int: 2,
                ..StackEffect::none()
            },
            "pushZeroInsets" => StackEffect {
                pushes_int: 4,
                ..StackEffect::none()
            },
            "pushFontMetrics" => StackEffect {
                pops_int: 1,
                pushes_int: 5,
                ..StackEffect::none()
            },
            "fullscreen_enter" => StackEffect {
                pops_int: 2,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "fullscreen_getmode" => StackEffect {
                pops_int: 1,
                pushes_int: 2,
                ..StackEffect::none()
            },
            "fullscreen_modecount" => StackEffect::int_push(1),
            "window_getinsets" => StackEffect {
                pushes_int: 4,
                ..StackEffect::none()
            },
            "targetmode_active" | "if_get_gamescreen" | "interface_getpickingradius" => {
                StackEffect::int_push(1)
            }
            "get_active_minimenu_entry" | "get_second_minimenu_entry" => StackEffect {
                pushes_int: 1,
                pushes_obj: 3,
                ..StackEffect::none()
            },
            "get_minimenu_target" => StackEffect {
                pushes_int: 1,
                pushes_obj: 2,
                ..StackEffect::none()
            },
            "if_hassub" | "if_getnextsubid" => StackEffect {
                pops_int: 1,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "if_hassubmodal" | "if_hassuboverlay" => StackEffect {
                pops_int: 2,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "setup_messagebox" => StackEffect {
                pops_int: 11,
                ..StackEffect::none()
            },
            "formatminimenu" => StackEffect {
                pops_int: 12,
                ..StackEffect::none()
            },
            "minimenuopen" => StackEffect {
                pops_int: 2,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "setsubmenuminlength" => StackEffect::int_pop(1),
            "if_set_gamescreen_enabled" | "interface_setpickingradius" | "if_closesubclient" => {
                StackEffect::int_pop(1)
            }
            "if_opensubclient" => StackEffect {
                pops_int: 2,
                ..StackEffect::none()
            },
            "opplayer" => StackEffect {
                pops_int: 1,
                pops_obj: 1,
                ..StackEffect::none()
            },
            "opplayert" => StackEffect {
                pops_obj: 1,
                ..StackEffect::none()
            },
            "defaultminimenu"
            | "minimenu_close"
            | "if_close"
            | "force_interface_drag"
            | "cancel_interface_drag"
            | "targetmode_cancel" => StackEffect::none(),
            "db_listall" => StackEffect {
                pops_int: 1,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "db_find" => StackEffect {
                pops_int: 2,
                ..StackEffect::none()
            },
            "db_find_with_count" | "db_getfieldcount" | "db_find_refine" => StackEffect {
                pops_int: 2,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "db_findnext" => StackEffect::int_push(1),
            // DB field arity depends on column schema. Keep validation
            // conservative by modeling one unknown result instead of none.
            "db_getfield" => StackEffect {
                pops_int: 3,
                pushes_unknown: 1,
                ..StackEffect::none()
            },
            "db_find_get" | "db_getrowtable" => StackEffect {
                pops_int: 1,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "notifications_sendlocal" => StackEffect {
                pops_int: 2,
                pops_obj: 2,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "notifications_sendgroupedlocal" => StackEffect {
                pops_int: 3,
                pops_obj: 3,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "notifications_cancellocal" => StackEffect::int_pop(1),
            "autosetup_dosetup" | "autosetup_dosetupstatus" => StackEffect {
                pushes_int: 2,
                ..StackEffect::none()
            },

            "cc_create" => StackEffect {
                pops_int: 3,
                ..StackEffect::none()
            },
            "cc_delete"
            | "cc_sendtofront"
            | "cc_sendtoback"
            | "cc_resetmodellighting"
            | "cc_clearops"
            | "cc_callonresize" => StackEffect::none(),
            "cc_deleteall" | "if_sendtofront" | "if_sendtoback" => StackEffect::int_pop(1),
            "cc_find" => StackEffect {
                pops_int: 2,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "if_find" => StackEffect {
                pops_int: 1,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "cc_settext" | "cc_setpausetext" => StackEffect {
                pops_obj: 1,
                ..StackEffect::none()
            },
            "if_settext" | "if_setpausetext" => StackEffect {
                pops_int: 1,
                pops_obj: 1,
                ..StackEffect::none()
            },
            "cc_setgraphic"
            | "cc_sethide"
            | "cc_setcolour"
            | "cc_setfill"
            | "cc_settrans"
            | "cc_setlinewid"
            | "cc_setmodel"
            | "cc_set2dangle"
            | "cc_settiling"
            | "cc_setmodelanim"
            | "cc_setmodelorthog"
            | "cc_setmodelzoom"
            | "cc_settextfont"
            | "cc_settextshadow"
            | "cc_settextantimacro"
            | "cc_setoutline"
            | "cc_setgraphicshadow"
            | "cc_setclickmask"
            | "cc_setheld"
            | "cc_setfontmono"
            | "cc_setnoclickthrough"
            | "cc_setstylesheet" => StackEffect::int_pop(1),
            "if_setgraphic"
            | "if_sethide"
            | "if_setcolour"
            | "if_setfill"
            | "if_settrans"
            | "if_setlinewid"
            | "if_setmodel"
            | "if_set2dangle"
            | "if_settiling"
            | "if_setmodelanim"
            | "if_setmodelorthog"
            | "if_setmodelzoom"
            | "if_settextfont"
            | "if_settextshadow"
            | "if_settextantimacro"
            | "if_setoutline"
            | "if_setgraphicshadow"
            | "if_setclickmask"
            | "if_setheld"
            | "if_setfontmono"
            | "if_setnoclickthrough"
            | "if_setstylesheet"
            | "if_resetmodellighting" => StackEffect {
                pops_int: 2,
                ..StackEffect::none()
            },
            "cc_setscrollpos" | "cc_setscrollsize" | "cc_setaspect" | "cc_setmodelorigin"
            | "cc_setparam" | "cc_setparam_int" => StackEffect {
                pops_int: 2,
                ..StackEffect::none()
            },
            "if_setscrollpos" | "if_setscrollsize" | "if_setaspect" | "if_setmodelorigin"
            | "if_setparam_int" => StackEffect {
                pops_int: 3,
                ..StackEffect::none()
            },
            "cc_setparam_string" => StackEffect {
                pops_int: 1,
                pops_obj: 1,
                ..StackEffect::none()
            },
            "if_setparam_string" => StackEffect {
                pops_int: 2,
                pops_obj: 1,
                ..StackEffect::none()
            },
            "cc_param" => StackEffect {
                pops_int: 1,
                pushes_unknown: 1,
                ..StackEffect::none()
            },
            "oc_param" | "nc_param" | "lc_param" | "struct_param" | "seq_param" | "mec_param"
            | "quest_param" => StackEffect {
                pops_int: 2,
                pushes_unknown: 1,
                ..StackEffect::none()
            },
            "enum_string" => StackEffect {
                pops_int: 2,
                pushes_obj: 1,
                ..StackEffect::none()
            },
            "enum" | "_enum" => StackEffect {
                pops_int: 4,
                pushes_unknown: 1,
                ..StackEffect::none()
            },
            "enum_hasoutput" | "enum_getreversecount" => StackEffect {
                pops_int: 3,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "enum_hasoutput_string" | "enum_getreversecount_string" => StackEffect {
                pops_int: 1,
                pops_obj: 1,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "enum_getoutputcount" => StackEffect::int_op(1),
            "enum_getreverseindex" => StackEffect {
                pops_int: 5,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "enum_getreverseindex_string" => StackEffect {
                pops_int: 3,
                pops_obj: 1,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "cc_settextalign" | "cc_setrecol" | "cc_setretex" => StackEffect {
                pops_int: 3,
                ..StackEffect::none()
            },
            "if_settextalign" | "if_setrecol" | "if_setretex" => StackEffect {
                pops_int: 4,
                ..StackEffect::none()
            },
            "cc_setposition" | "cc_setsize" | "cc_setmodeltint" => StackEffect {
                pops_int: 4,
                ..StackEffect::none()
            },
            "if_setposition" | "if_setsize" | "if_setmodeltint" => StackEffect {
                pops_int: 5,
                ..StackEffect::none()
            },
            "cc_setmodelangle" => StackEffect {
                pops_int: 6,
                ..StackEffect::none()
            },
            "if_setmodelangle" => StackEffect {
                pops_int: 7,
                ..StackEffect::none()
            },
            "cc_setmodellighting" => StackEffect {
                pops_int: 10,
                ..StackEffect::none()
            },
            "if_setmodellighting" => StackEffect {
                pops_int: 11,
                ..StackEffect::none()
            },
            "quickchat_dynamic_command_add" => StackEffect::int_op(2),

            // ── Engine commands (cc_*, if_*): conservatively assume int args ──
            _ if cmd.starts_with("cc_") || cmd.starts_with("if_") => StackEffect {
                pops_int: 2,
                ..StackEffect::none()
            },

            // ── Fallback patterns ──
            _ if cmd.starts_with("push_") => StackEffect::int_push(1),
            _ if cmd.starts_with("pop_") => StackEffect::int_pop(1),
            _ if cmd.starts_with("branch_") => StackEffect::int_pop(1),
            _ if cmd.starts_with("long_branch_") => StackEffect {
                pops_long: 1,
                ..StackEffect::none()
            },
            // Unmodelled command: use the client-extracted opcode stack-effect
            // table so the validator's typed stacks track getters/config lookups/
            // value ops (e.g. map_members, oc_name) instead of treating them as
            // no-ops and falsely reporting StackUnderflow.
            _ => crate::transpile::opcode_stack_effect(cmd).map_or_else(StackEffect::none, |e| {
                StackEffect {
                    pops_int: e.int_pops,
                    pops_obj: e.obj_pops,
                    pops_long: e.long_pops,
                    pushes_int: e.int_pushes,
                    pushes_obj: e.obj_pushes,
                    pushes_long: e.long_pushes,
                    pushes_unknown: 0,
                }
            }),
        }
    }

    /// Resolve a varp reference to determine which stack `push_var`/`pop_var` affects.
    fn varp_stack_effect_for(&self, operand: &Operand, is_push: bool) -> StackEffect {
        if let Operand::VarRef(vr) = operand {
            let type_id = self
                .ctx
                .varps_by_domain
                .get(&vr.domain)
                .and_then(|vars| vars.get(&u32::from(vr.id)))
                .and_then(|v| v.type_id);
            match type_id {
                // type_id 1 = long, 2 = string, 0/None/N = int
                Some(1) if is_push => StackEffect::long_push(1),
                Some(1) => StackEffect {
                    pops_long: 1,
                    ..StackEffect::none()
                },
                Some(2) if is_push => StackEffect::obj_push(1),
                Some(2) => StackEffect {
                    pops_obj: 1,
                    ..StackEffect::none()
                },
                _ if is_push => StackEffect::int_push(1),
                _ => StackEffect::int_pop(1),
            }
        } else if is_push {
            StackEffect::int_push(1)
        } else {
            StackEffect::int_pop(1)
        }
    }

    fn gosub_stack_effect(
        &self,
        operand: &Operand,
        script_catalog: &crate::transpile::ScriptCatalog,
        script_signatures: &std::collections::HashMap<
            crate::transpile::ScriptId,
            crate::transpile::ScriptSignature,
        >,
    ) -> StackEffect {
        let Operand::Script(raw_id) = operand else {
            return StackEffect::none();
        };
        let Some((_metadata, signature)) = crate::transpile::resolve_call_target_signature(
            script_catalog,
            script_signatures,
            *raw_id,
        ) else {
            return StackEffect::none();
        };

        StackEffect {
            pops_int: usize::from(signature.arg_count_int),
            pops_obj: usize::from(signature.arg_count_obj),
            pops_long: usize::from(signature.arg_count_long),
            pushes_int: usize::from(signature.return_type != "void"),
            pushes_obj: 0,
            pushes_long: 0,
            pushes_unknown: 0,
        }
    }
}

pub fn build_validation_catalog(
    ctx: &ResolverContext,
    extra_scripts: &[(u32, Vec<u8>)],
) -> ScriptCatalog {
    let empty_group_names = HashMap::<u32, String>::new();
    if extra_scripts.is_empty() {
        return build_script_catalog(
            &ctx.scripts,
            &empty_group_names,
            &ctx.opcode_book,
            ctx.build,
        );
    }
    let mut merged_scripts = ctx.scripts.clone();
    for (packed_id, bytes) in extra_scripts {
        merged_scripts.insert(*packed_id, bytes.clone());
    }
    build_script_catalog(
        &merged_scripts,
        &empty_group_names,
        &ctx.opcode_book,
        ctx.build,
    )
}

pub fn extend_validation_catalog(
    base_catalog: &ScriptCatalog,
    opcode_book: &OpcodeBook,
    build: u32,
    extra_scripts: &[(u32, &[u8])],
) -> ScriptCatalog {
    if extra_scripts.is_empty() {
        return base_catalog.clone();
    }

    let empty_group_names = HashMap::<u32, String>::new();
    let mut builder = ScriptCatalogBuilder::new(&empty_group_names, opcode_book, build);
    for (packed_id, bytes) in extra_scripts {
        builder.add_script(*packed_id, bytes);
    }

    let mut catalog = base_catalog.clone();
    let overlay_catalog = builder.build();
    for metadata in overlay_catalog.iter() {
        catalog.insert(metadata.clone());
    }
    catalog
}

fn missing_script_report(script_id: u32, build: u32) -> ValidationReport {
    let mut report = ValidationReport {
        script_id,
        script_name: None,
        build,
        instruction_count: 0,
        errors: Vec::new(),
        warnings: Vec::new(),
    };
    report.errors.push(ValidationError::ScriptNotFound {
        index: 0,
        called_id: script_id as i32,
    });
    report
}

// ── Batch validation ──

#[derive(Debug, Clone, Serialize)]
pub struct BatchReport {
    pub build: u32,
    pub scripts_validated: usize,
    pub scripts_with_errors: usize,
    pub total_errors: usize,
    pub results: Vec<ValidationReport>,
}

pub fn validate_scripts(ctx: &ResolverContext, script_ids: &[u32]) -> BatchReport {
    let validator = Cs2Validator::new(ctx);
    let mut results = Vec::new();
    let mut scripts_with_errors = 0;
    let mut total_errors = 0;

    for &id in script_ids {
        let report = validator.validate(id);
        if !report.is_valid() {
            scripts_with_errors += 1;
        }
        total_errors += report.errors.len();
        results.push(report);
    }

    BatchReport {
        build: ctx.build,
        scripts_validated: results.len(),
        scripts_with_errors,
        total_errors,
        results,
    }
}

#[cfg(test)]
mod tests {
    use super::{Cs2Validator, ResolverContext, ValidationError};
    use crate::script::{CompiledScript, Instruction, OpcodeBook, Operand, encode_script};
    use crate::vars::{VarDomain, VarEntry};
    use std::collections::{BTreeMap, HashMap};
    use std::path::PathBuf;

    fn test_data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data")
    }

    fn build_ctx(
        build: u32,
        scripts: &[(u32, CompiledScript)],
    ) -> crate::error::Result<ResolverContext> {
        let opcode_book = OpcodeBook::load(&test_data_dir(), build, 0)?;
        let mut raw_scripts = BTreeMap::new();
        let mut decoded_scripts = BTreeMap::new();
        for (script_id, script) in scripts {
            raw_scripts.insert(*script_id, encode_script(script, &opcode_book, build)?);
            decoded_scripts.insert(*script_id, script.clone());
        }

        Ok(ResolverContext {
            build,
            opcode_book,
            interfaces: BTreeMap::new(),
            scripts: raw_scripts,
            varps_by_domain: HashMap::new(),
            varbits: BTreeMap::new(),
            params: BTreeMap::new(),
            enums: BTreeMap::new(),
            structs: BTreeMap::new(),
            decoded_scripts,
            parsed_components: BTreeMap::new(),
            npcs: BTreeMap::new(),
            objs: BTreeMap::new(),
            locs: BTreeMap::new(),
            seqs: BTreeMap::new(),
            spots: BTreeMap::new(),
            invs: BTreeMap::new(),
            dbtables: BTreeMap::new(),
            dbrows: BTreeMap::new(),
        })
    }

    #[test]
    fn gosub_stack_effect_resolves_group_id_void_callee() -> crate::error::Result<()> {
        const BUILD: u32 = 910;
        let void_callee_id = 100_u32 << 16;
        let void_entry_id = 200_u32 << 16;

        let void_callee = CompiledScript {
            name: Some("[proc,callee_void]".to_string()),
            local_count_int: 0,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 2,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Byte(0),
            }],
        };

        let void_entry = CompiledScript {
            name: Some("[proc,caller_void]".to_string()),
            local_count_int: 1,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(1),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(2),
                },
                Instruction {
                    opcode: 0,
                    command: "gosub_with_params".to_string(),
                    operand: Operand::Script(100),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "return".to_string(),
                    operand: Operand::Byte(0),
                },
            ],
        };

        let ctx = build_ctx(
            BUILD,
            &[(void_callee_id, void_callee), (void_entry_id, void_entry)],
        )?;
        let report = Cs2Validator::new(&ctx).validate(void_entry_id);

        assert!(
            report
                .warnings
                .iter()
                .all(|warning| !warning.contains("not found")),
            "unexpected warnings: {:?}",
            report.warnings
        );
        assert!(report.errors.iter().any(|error| matches!(
            error,
            ValidationError::StackUnderflow { index: 3, stack, .. } if stack == "int"
        )));
        Ok(())
    }

    #[test]
    fn gosub_stack_effect_resolves_group_id_value_callee() -> crate::error::Result<()> {
        const BUILD: u32 = 910;
        let value_callee_id = 101_u32 << 16;
        let value_entry_id = 201_u32 << 16;

        let value_callee = CompiledScript {
            name: Some("[proc,callee_value]".to_string()),
            local_count_int: 0,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 2,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(7),
                },
                Instruction {
                    opcode: 0,
                    command: "return".to_string(),
                    operand: Operand::Byte(0),
                },
            ],
        };

        let value_entry = CompiledScript {
            name: Some("[proc,caller_value]".to_string()),
            local_count_int: 1,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(1),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(2),
                },
                Instruction {
                    opcode: 0,
                    command: "gosub_with_params".to_string(),
                    operand: Operand::Script(101),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "return".to_string(),
                    operand: Operand::Byte(0),
                },
            ],
        };

        let ctx = build_ctx(
            BUILD,
            &[
                (value_callee_id, value_callee),
                (value_entry_id, value_entry),
            ],
        )?;
        let report = Cs2Validator::new(&ctx).validate(value_entry_id);

        assert!(
            report
                .warnings
                .iter()
                .all(|warning| !warning.contains("not found")),
            "unexpected warnings: {:?}",
            report.warnings
        );
        assert!(
            !report
                .errors
                .iter()
                .any(|error| matches!(error, ValidationError::StackUnderflow { .. })),
            "unexpected errors: {:?}",
            report.errors
        );
        Ok(())
    }

    #[test]
    fn oc_param_unknown_result_can_flow_to_object_stack() -> crate::error::Result<()> {
        const BUILD: u32 = 910;
        let script_id = 300_u32 << 16;

        let script = CompiledScript {
            name: Some("[proc,oc_param_string]".to_string()),
            local_count_int: 0,
            local_count_object: 1,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(1),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(2),
                },
                Instruction {
                    opcode: 0,
                    command: "oc_param".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_string_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "return".to_string(),
                    operand: Operand::Byte(0),
                },
            ],
        };

        let ctx = build_ctx(BUILD, &[(script_id, script)])?;
        let report = Cs2Validator::new(&ctx).validate(script_id);

        assert!(
            !report
                .errors
                .iter()
                .any(|error| matches!(error, ValidationError::StackUnderflow { .. })),
            "unexpected errors: {:?}",
            report.errors
        );
        Ok(())
    }

    #[test]
    fn enum_unknown_result_can_flow_to_int_stack() -> crate::error::Result<()> {
        const BUILD: u32 = 910;
        let script_id = 301_u32 << 16;

        let script = CompiledScript {
            name: Some("[proc,enum_value]".to_string()),
            local_count_int: 1,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(105),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(10),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(20),
                },
                Instruction {
                    opcode: 0,
                    command: "_enum".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "return".to_string(),
                    operand: Operand::Byte(0),
                },
            ],
        };

        let ctx = build_ctx(BUILD, &[(script_id, script)])?;
        let report = Cs2Validator::new(&ctx).validate(script_id);

        assert!(
            !report
                .errors
                .iter()
                .any(|error| matches!(error, ValidationError::StackUnderflow { .. })),
            "unexpected errors: {:?}",
            report.errors
        );
        Ok(())
    }

    #[test]
    fn fixed_domain_var_refs_use_config_types_for_stack_validation() -> crate::error::Result<()> {
        const BUILD: u32 = 718;
        let script_id = 302_u32 << 16;

        let script = CompiledScript {
            name: Some("[proc,fixed_domain_var_refs]".to_string()),
            local_count_int: 0,
            local_count_object: 1,
            local_count_long: 1,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![
                Instruction {
                    opcode: 0,
                    command: "push_constant_string".to_string(),
                    operand: Operand::Str("x".to_string()),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_varc_string".to_string(),
                    operand: Operand::VarRef(crate::script::VarRef {
                        domain: VarDomain::Client,
                        id: 9,
                        transmog: false,
                    }),
                },
                Instruction {
                    opcode: 0,
                    command: "push_varc_string".to_string(),
                    operand: Operand::VarRef(crate::script::VarRef {
                        domain: VarDomain::Client,
                        id: 9,
                        transmog: false,
                    }),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_string_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_varclansetting_long".to_string(),
                    operand: Operand::VarRef(crate::script::VarRef {
                        domain: VarDomain::ClanSetting,
                        id: 77,
                        transmog: false,
                    }),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_long_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "return".to_string(),
                    operand: Operand::Byte(0),
                },
            ],
        };

        let mut ctx = build_ctx(BUILD, &[(script_id, script)])?;
        ctx.varps_by_domain.insert(
            VarDomain::Client,
            BTreeMap::from([(
                9_u32,
                VarEntry {
                    id: 9,
                    domain: VarDomain::Client,
                    var_name: "varclient_9".to_string(),
                    type_id: Some(2),
                    lifetime: None,
                    transmit_level: None,
                    client_code: None,
                    domain_default: true,
                    wiki_sync: false,
                },
            )]),
        );
        ctx.varps_by_domain.insert(
            VarDomain::ClanSetting,
            BTreeMap::from([(
                77_u32,
                VarEntry {
                    id: 77,
                    domain: VarDomain::ClanSetting,
                    var_name: "varclansetting_77".to_string(),
                    type_id: Some(1),
                    lifetime: None,
                    transmit_level: None,
                    client_code: None,
                    domain_default: true,
                    wiki_sync: false,
                },
            )]),
        );

        let report = Cs2Validator::new(&ctx).validate(script_id);
        assert!(
            !report
                .errors
                .iter()
                .any(|error| matches!(error, ValidationError::StackUnderflow { .. })),
            "unexpected errors: {:?}",
            report.errors
        );
        Ok(())
    }

    #[test]
    fn typed_constant_push_resolves_stack_type_from_operand() -> crate::error::Result<()> {
        const BUILD: u32 = 910;
        let script_id = 304_u32 << 16;

        // push_constant_string is the typed-constant opcode: an int-tagged
        // constant feeds the int stack, so the following pop_int_local must not
        // underflow. Mirror the same for the long-tagged form.
        let script = CompiledScript {
            name: Some("[proc,typed_constant_push]".to_string()),
            local_count_int: 1,
            local_count_object: 0,
            local_count_long: 1,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![
                Instruction {
                    opcode: 0,
                    command: "push_constant_string".to_string(),
                    operand: Operand::Int(5),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_string".to_string(),
                    operand: Operand::Long(7),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_long_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "return".to_string(),
                    operand: Operand::Byte(0),
                },
            ],
        };

        let ctx = build_ctx(BUILD, &[(script_id, script)])?;
        let report = Cs2Validator::new(&ctx).validate(script_id);
        assert!(
            !report
                .errors
                .iter()
                .any(|error| matches!(error, ValidationError::StackUnderflow { .. })),
            "int/long-tagged typed constants should feed the matching stack: {:?}",
            report.errors
        );
        Ok(())
    }

    #[test]
    fn clan_and_player_group_helpers_use_source_backed_stack_effects() -> crate::error::Result<()> {
        const BUILD: u32 = 910;
        let script_id = 303_u32 << 16;

        let script = CompiledScript {
            name: Some("[proc,clan_player_group_helpers]".to_string()),
            local_count_int: 0,
            local_count_object: 1,
            local_count_long: 1,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![
                Instruction {
                    opcode: 0,
                    command: "activeclansettings_getclanname".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_string_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(2),
                },
                Instruction {
                    opcode: 0,
                    command: "activeclanchannel_getuserdisplayname".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_string_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(1),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(1),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(2),
                },
                Instruction {
                    opcode: 0,
                    command: "player_group_member_get_same_world_var".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_long_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "return".to_string(),
                    operand: Operand::Byte(0),
                },
            ],
        };

        let ctx = build_ctx(BUILD, &[(script_id, script)])?;
        let report = Cs2Validator::new(&ctx).validate(script_id);

        assert!(
            !report
                .errors
                .iter()
                .any(|error| matches!(error, ValidationError::StackUnderflow { .. })),
            "unexpected errors: {:?}",
            report.errors
        );
        Ok(())
    }

    #[test]
    fn login_shop_notification_and_detail_helpers_use_exact_stack_effects()
    -> crate::error::Result<()> {
        const BUILD: u32 = 910;
        let script_id = 304_u32 << 16;

        let script = CompiledScript {
            name: Some("[proc,login_shop_notification_detail_helpers]".to_string()),
            local_count_int: 3,
            local_count_object: 2,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![
                Instruction {
                    opcode: 0,
                    command: "create_get_email".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_string_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_string".to_string(),
                    operand: Operand::Str("title".to_string()),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_string".to_string(),
                    operand: Operand::Str("body".to_string()),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(1),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(2),
                },
                Instruction {
                    opcode: 0,
                    command: "notifications_sendlocal".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(5),
                },
                Instruction {
                    opcode: 0,
                    command: "shop_getcategorydescription".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_string_local".to_string(),
                    operand: Operand::Local(1),
                },
                Instruction {
                    opcode: 0,
                    command: "autosetup_dosetupstatus".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(1),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(2),
                },
                Instruction {
                    opcode: 0,
                    command: "login_resetreply".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "return".to_string(),
                    operand: Operand::Byte(0),
                },
            ],
        };

        let ctx = build_ctx(BUILD, &[(script_id, script)])?;
        let report = Cs2Validator::new(&ctx).validate(script_id);

        assert!(
            !report
                .errors
                .iter()
                .any(|error| matches!(error, ValidationError::StackUnderflow { .. })),
            "unexpected errors: {:?}",
            report.errors
        );
        Ok(())
    }

    #[test]
    fn db_helpers_use_source_backed_stack_effects() -> crate::error::Result<()> {
        const BUILD: u32 = 910;
        let script_id = 305_u32 << 16;

        let script = CompiledScript {
            name: Some("[proc,db_helper_stack_effects]".to_string()),
            local_count_int: 2,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(10),
                },
                Instruction {
                    opcode: 0,
                    command: "db_listall".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(20),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(30),
                },
                Instruction {
                    opcode: 0,
                    command: "db_find".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(20),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(30),
                },
                Instruction {
                    opcode: 0,
                    command: "db_find_with_count".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(1),
                },
                Instruction {
                    opcode: 0,
                    command: "db_findnext".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(1),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(2),
                },
                Instruction {
                    opcode: 0,
                    command: "db_getfieldcount".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(1),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(0),
                },
                Instruction {
                    opcode: 0,
                    command: "db_find_get".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(1),
                },
                Instruction {
                    opcode: 0,
                    command: "db_getrowtable".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(1),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(11),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(22),
                },
                Instruction {
                    opcode: 0,
                    command: "db_find_refine".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "return".to_string(),
                    operand: Operand::Byte(0),
                },
            ],
        };

        let ctx = build_ctx(BUILD, &[(script_id, script)])?;
        let report = Cs2Validator::new(&ctx).validate(script_id);

        assert!(
            report.errors.is_empty(),
            "unexpected validation errors: {:?}",
            report.errors
        );
        Ok(())
    }

    #[test]
    fn minimenu_and_messagebox_helpers_use_source_backed_stack_effects() -> crate::error::Result<()>
    {
        const BUILD: u32 = 910;
        let script_id = 306_u32 << 16;

        let mut code = Vec::new();
        for value in 0..11 {
            code.push(Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(value),
            });
        }
        code.push(Instruction {
            opcode: 0,
            command: "setup_messagebox".to_string(),
            operand: Operand::Byte(0),
        });
        for value in 0..12 {
            code.push(Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(value),
            });
        }
        code.push(Instruction {
            opcode: 0,
            command: "formatminimenu".to_string(),
            operand: Operand::Byte(0),
        });
        code.extend([
            Instruction {
                opcode: 0,
                command: "get_active_minimenu_entry".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(2),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "get_second_minimenu_entry".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(2),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "get_minimenu_length".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "get_minimenu_target".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(2),
            },
            Instruction {
                opcode: 0,
                command: "get_mousebuttons".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(1),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(2),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(10),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(20),
            },
            Instruction {
                opcode: 0,
                command: "minimenuopen".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: Operand::Local(0),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(5),
            },
            Instruction {
                opcode: 0,
                command: "setsubmenuminlength".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: Operand::Str("Attack".to_string()),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: Operand::Int(1),
            },
            Instruction {
                opcode: 0,
                command: "opplayer".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: Operand::Str("Use".to_string()),
            },
            Instruction {
                opcode: 0,
                command: "opplayert".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "defaultminimenu".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "minimenu_close".to_string(),
                operand: Operand::Byte(0),
            },
            Instruction {
                opcode: 0,
                command: "return".to_string(),
                operand: Operand::Byte(0),
            },
        ]);

        let script = CompiledScript {
            name: Some("[proc,minimenu_messagebox_helper_stack_effects]".to_string()),
            local_count_int: 3,
            local_count_object: 3,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code,
        };

        let ctx = build_ctx(BUILD, &[(script_id, script)])?;
        let report = Cs2Validator::new(&ctx).validate(script_id);

        assert!(
            report.errors.is_empty(),
            "unexpected validation errors: {:?}",
            report.errors
        );
        Ok(())
    }

    #[test]
    fn interface_misc_helpers_use_source_backed_stack_effects() -> crate::error::Result<()> {
        const BUILD: u32 = 910;
        let script_id = 307_u32 << 16;

        let script = CompiledScript {
            name: Some("[proc,interface_misc_helper_stack_effects]".to_string()),
            local_count_int: 4,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![
                Instruction {
                    opcode: 0,
                    command: "window_getinsets".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(1),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(2),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(3),
                },
                Instruction {
                    opcode: 0,
                    command: "targetmode_active".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(100),
                },
                Instruction {
                    opcode: 0,
                    command: "if_hassub".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(1),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(100),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(7),
                },
                Instruction {
                    opcode: 0,
                    command: "if_hassubmodal".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(2),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(100),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(8),
                },
                Instruction {
                    opcode: 0,
                    command: "if_hassuboverlay".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(3),
                },
                Instruction {
                    opcode: 0,
                    command: "if_get_gamescreen".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(1),
                },
                Instruction {
                    opcode: 0,
                    command: "if_set_gamescreen_enabled".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(25),
                },
                Instruction {
                    opcode: 0,
                    command: "interface_setpickingradius".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "interface_getpickingradius".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(1),
                },
                Instruction {
                    opcode: 0,
                    command: "if_close".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(200),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(300),
                },
                Instruction {
                    opcode: 0,
                    command: "if_opensubclient".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(200),
                },
                Instruction {
                    opcode: 0,
                    command: "if_closesubclient".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "force_interface_drag".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "cancel_interface_drag".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "targetmode_cancel".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "return".to_string(),
                    operand: Operand::Byte(0),
                },
            ],
        };

        let ctx = build_ctx(BUILD, &[(script_id, script)])?;
        let report = Cs2Validator::new(&ctx).validate(script_id);

        assert!(
            report.errors.is_empty(),
            "unexpected validation errors: {:?}",
            report.errors
        );
        Ok(())
    }

    #[test]
    fn display_helpers_use_source_backed_stack_effects() -> crate::error::Result<()> {
        const BUILD: u32 = 910;
        let script_id = 308_u32 << 16;

        let script = CompiledScript {
            name: Some("[proc,display_helper_stack_effects]".to_string()),
            local_count_int: 5,
            local_count_object: 0,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![
                Instruction {
                    opcode: 0,
                    command: "pushCanvasSize".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(1),
                },
                Instruction {
                    opcode: 0,
                    command: "pushZeroInsets".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(1),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(2),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(3),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(7),
                },
                Instruction {
                    opcode: 0,
                    command: "pushFontMetrics".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(1),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(2),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(3),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(4),
                },
                Instruction {
                    opcode: 0,
                    command: "viewport_geteffectivesize".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(1),
                },
                Instruction {
                    opcode: 0,
                    command: "viewport_getzoom".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(2),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(3),
                },
                Instruction {
                    opcode: 0,
                    command: "viewport_getfov".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(1),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(800),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(600),
                },
                Instruction {
                    opcode: 0,
                    command: "fullscreen_enter".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(2),
                },
                Instruction {
                    opcode: 0,
                    command: "fullscreen_modecount".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(3),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(0),
                },
                Instruction {
                    opcode: 0,
                    command: "fullscreen_getmode".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(1),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(123),
                },
                Instruction {
                    opcode: 0,
                    command: "if_getnextsubid".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(4),
                },
                Instruction {
                    opcode: 0,
                    command: "return".to_string(),
                    operand: Operand::Byte(0),
                },
            ],
        };

        let ctx = build_ctx(BUILD, &[(script_id, script)])?;
        let report = Cs2Validator::new(&ctx).validate(script_id);

        assert!(
            report.errors.is_empty(),
            "unexpected validation errors: {:?}",
            report.errors
        );
        Ok(())
    }

    #[test]
    fn worldlist_helpers_use_source_backed_stack_effects() -> crate::error::Result<()> {
        const BUILD: u32 = 910;
        let script_id = 309_u32 << 16;

        let script = CompiledScript {
            name: Some("[proc,worldlist_helper_stack_effects]".to_string()),
            local_count_int: 9,
            local_count_object: 6,
            local_count_long: 0,
            argument_count_int: 0,
            argument_count_object: 0,
            argument_count_long: 0,
            code: vec![
                Instruction {
                    opcode: 0,
                    command: "worldlist_start".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_string_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(1),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_string_local".to_string(),
                    operand: Operand::Local(1),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(2),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_string_local".to_string(),
                    operand: Operand::Local(2),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(3),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(4),
                },
                Instruction {
                    opcode: 0,
                    command: "worldlist_next".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_string_local".to_string(),
                    operand: Operand::Local(3),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(5),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(6),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_string_local".to_string(),
                    operand: Operand::Local(4),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(7),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_string_local".to_string(),
                    operand: Operand::Local(5),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(8),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(302),
                },
                Instruction {
                    opcode: 0,
                    command: "worldlist_specific".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_string_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(1),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_string_local".to_string(),
                    operand: Operand::Local(1),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(2),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_string_local".to_string(),
                    operand: Operand::Local(2),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(3),
                },
                Instruction {
                    opcode: 0,
                    command: "worldlist_fetch".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(4),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_string".to_string(),
                    operand: Operand::Str("world302.example".to_string()),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(302),
                },
                Instruction {
                    opcode: 0,
                    command: "worldlist_switch".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(5),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(3),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(1),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(4),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(0),
                },
                Instruction {
                    opcode: 0,
                    command: "worldlist_sort".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "worldlist_autoworld".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "push_constant_int".to_string(),
                    operand: Operand::Int(1),
                },
                Instruction {
                    opcode: 0,
                    command: "worldlist_pingworlds".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "worldlist_specific_thisworld".to_string(),
                    operand: Operand::Byte(0),
                },
                Instruction {
                    opcode: 0,
                    command: "pop_int_local".to_string(),
                    operand: Operand::Local(6),
                },
                Instruction {
                    opcode: 0,
                    command: "return".to_string(),
                    operand: Operand::Byte(0),
                },
            ],
        };

        let ctx = build_ctx(BUILD, &[(script_id, script)])?;
        let report = Cs2Validator::new(&ctx).validate(script_id);

        assert!(
            report.errors.is_empty(),
            "unexpected validation errors: {:?}",
            report.errors
        );
        Ok(())
    }
}
