//! Per-opcode stack-effect computation for [`Cs2Validator`].
//!
//! A second inherent `impl<'a> Cs2Validator<'a>` block holding the giant
//! `stack_effect_for` opcode table plus the varp and gosub stack-effect helpers.
//! Split out of `validate.rs` (behavior-preserving) to keep the validator core small.

use super::{Cs2Validator, StackEffect};
use crate::script::Operand;

impl Cs2Validator<'_> {
    // Based on Ignis ClientScriptState runtime: three typed stacks
    // (intStack/isp, objectStack/osp, longStack/lsp).
    pub(super) fn stack_effect_for(
        &self,
        instr: &crate::script::Instruction,
        script_catalog: &crate::transpile::ScriptCatalog,
        script_signatures: &std::collections::HashMap<
            crate::transpile::ScriptId,
            crate::transpile::ScriptSignature,
        >,
    ) -> StackEffect {
        let cmd = &instr.command;
        let source_effect = crate::transpile::opcode_stack_effect_for_build(cmd, self.ctx.build);
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
            "branch_if_true" | "branch_if_false" => StackEffect::int_pop(1),
            "branch_not"
            | "branch_equals"
            | "branch_less_than"
            | "branch_greater_than"
            | "branch_less_than_or_equals"
            | "branch_greater_than_or_equals" => StackEffect {
                pops_int: 2,
                ..StackEffect::none()
            },
            "long_branch_not"
            | "long_branch_equals"
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
            "define_array" => StackEffect::int_pop(1),
            "array_sort" => StackEffect::int_pop(3),
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
            "activeclanchannel_find_affined" | "activeclansettings_find_affined"
                if self.ctx.build >= 938 =>
            {
                StackEffect {
                    pops_int: 1,
                    pushes_int: 1,
                    ..StackEffect::none()
                }
            }
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
            "db_find" if self.ctx.build >= 919 => StackEffect {
                pops_int: 3,
                ..StackEffect::none()
            },
            "db_find" => StackEffect {
                pops_int: 2,
                ..StackEffect::none()
            },
            "db_find_with_count" | "db_find_refine" if self.ctx.build >= 919 => StackEffect {
                pops_int: 3,
                pushes_int: 1,
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
            "db_filter_value" => StackEffect {
                pops_int: 4,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "db_filter_find" => StackEffect {
                pops_int: 5,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "db_filter_unknown" => StackEffect {
                pops_int: 1,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "db_filter_combine" => StackEffect {
                pops_int: 2,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "db_filter_substring" => StackEffect {
                pops_int: 2,
                pops_obj: 1,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "db_filter_column" => StackEffect {
                pops_int: 3,
                pushes_int: 1,
                ..StackEffect::none()
            },
            "cam2_setlookatmode" | "cam2_setpositionmode" => StackEffect::int_pop(1),
            "cam2_setpositionentity_npc" | "cam2_setpositionentity_player"
                if self.ctx.build >= 919 =>
            {
                StackEffect {
                    pops_int: 8,
                    ..StackEffect::none()
                }
            }
            "cam2_setpositionentity_npc" | "cam2_setpositionentity_player" => StackEffect {
                pops_int: 7,
                ..StackEffect::none()
            },
            "error" => StackEffect {
                pops_obj: 1,
                ..StackEffect::none()
            },
            "store_lookup" => StackEffect {
                pops_int: 1,
                pops_obj: 1,
                pushes_int: 10,
                pushes_long: 3,
                ..StackEffect::none()
            },
            "field6563" => StackEffect::int_push(1),
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
            "cc_createchild" => StackEffect {
                pops_int: 3,
                ..StackEffect::none()
            },
            "if_createchild" => StackEffect {
                pops_int: 4,
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
            "cc_npc_setcustombodymodel"
            | "cc_npc_setcustomheadmodel"
            | "cc_npc_setcustomrecol"
            | "cc_npc_setcustomretex" => StackEffect::int_pop(2),
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
            "if_npc_setcustombodymodel"
            | "if_npc_setcustomheadmodel"
            | "if_npc_setcustomrecol"
            | "if_npc_setcustomretex" => StackEffect::int_pop(3),
            "cc_setscrollpos" | "cc_setscrollsize" | "cc_setaspect" | "cc_setmodelorigin"
            | "cc_setparam" | "cc_setparam_int" => StackEffect {
                pops_int: 2,
                ..StackEffect::none()
            },
            "cc_setcustombodyretex"
            | "cc_setcustombodyrecol"
            | "cc_setcustomheadretex"
            | "cc_setcustomheadrecol" => StackEffect::int_pop(4),
            "if_setcustombodyretex"
            | "if_setcustombodyrecol"
            | "if_setcustomheadretex"
            | "if_setcustomheadrecol" => StackEffect::int_pop(5),
            "cc_npc_setcustombodymodel_transformed" => StackEffect::int_pop(10),
            "if_npc_setcustombodymodel_transformed" => StackEffect::int_pop(11),
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

            // Unmodelled command: use the client-extracted opcode stack-effect
            // table so the validator's typed stacks track getters/config lookups/
            // value ops (e.g. map_members, oc_name) instead of treating them as
            // no-ops and falsely reporting StackUnderflow.
            _ if source_effect.is_some() => {
                let e = source_effect.expect("checked above");
                StackEffect {
                    pops_int: e.int_pops,
                    pops_obj: e.obj_pops,
                    pops_long: e.long_pops,
                    pushes_int: e.int_pushes,
                    pushes_obj: e.obj_pushes,
                    pushes_long: e.long_pushes,
                    pushes_unknown: 0,
                }
            }

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
            _ => StackEffect::none(),
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
            pushes_int: usize::from(signature.return_count_int)
                + usize::from(
                    signature.total_returns() == 0
                        && signature
                            .return_type
                            .split('|')
                            .any(|part| matches!(part.trim(), "number" | "boolean" | "unknown")),
                ),
            pushes_obj: usize::from(signature.return_count_obj)
                + usize::from(
                    signature.total_returns() == 0
                        && signature
                            .return_type
                            .split('|')
                            .any(|part| part.trim() == "string"),
                ),
            pushes_long: usize::from(signature.return_count_long)
                + usize::from(
                    signature.total_returns() == 0
                        && signature
                            .return_type
                            .split('|')
                            .any(|part| part.trim() == "bigint"),
                ),
            pushes_unknown: 0,
        }
    }
}
