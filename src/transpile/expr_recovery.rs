use super::ast::{
    BinaryOp, CallExpr, CallbackLiteral, Expression, Identifier, InstructionNode, NumberLiteral,
    OperandNode, StringLiteral,
};
use crate::vars::VarDomain;

pub struct StackEffect {
    pub pops: usize,
    pub pushes: usize,
}

const MAX_JOIN_STRING_PARTS: usize = 1024;
const MAX_CALLBACK_WATCHERS: usize = 4096;

/// Classifies opcode commands by semantic prefix for dispatch in
/// catch-all arms. Specific opcodes are matched by exact name first.
enum OpcodeCategory {
    Push,
    Pop,
    Branch,
    CC,
    IF,
    Other,
}

fn categorize(cmd: &str) -> OpcodeCategory {
    if cmd.starts_with("push_") {
        OpcodeCategory::Push
    } else if cmd.starts_with("pop_") {
        OpcodeCategory::Pop
    } else if cmd.starts_with("branch_") || cmd.starts_with("long_branch_") {
        OpcodeCategory::Branch
    } else if cmd.starts_with("cc_") {
        OpcodeCategory::CC
    } else if cmd.starts_with("if_") {
        OpcodeCategory::IF
    } else {
        OpcodeCategory::Other
    }
}

fn stack_effect(
    cmd: &str,
    operand: &OperandNode,
    script_catalog: &super::ScriptCatalog,
    script_signatures: &std::collections::HashMap<super::ScriptId, super::ScriptSignature>,
) -> StackEffect {
    match cmd {
        // Push: pops 0, pushes 1
        "push_constant_int"
        | "push_long_constant"
        | "push_constant_string"
        | "push_var"
        | "push_varbit"
        | "push_int_local"
        | "push_string_local"
        | "push_long_local"
        | "push_varc_int"
        | "push_varc_string"
        | "push_varclan"
        | "push_varclanbit"
        | "push_varclan_long"
        | "push_varclan_string"
        | "push_varclansetting"
        | "push_varclansettingbit"
        | "push_varclansetting_long"
        | "push_varclansetting_string" => StackEffect { pops: 0, pushes: 1 },

        // Array push: pop index, push value
        "push_array_int" | "push_array_string" => StackEffect { pops: 1, pushes: 1 },
        "push_array_int_leave_index_on_stack" | "push_array_int_and_index" => {
            StackEffect { pops: 1, pushes: 2 }
        }

        // Pop/discard: pops 1, pushes 0 (`pop_varbit` is a store, not a push)
        "pop_int_local" | "pop_string_local" | "pop_long_local" | "pop_var" | "pop_varbit"
        | "pop_varc_int" | "pop_varc_string" | "pop_int_discard" | "pop_string_discard"
        | "pop_long_discard" => StackEffect { pops: 1, pushes: 0 },

        // Array pop: pop value, pop index, store
        "pop_array_int" | "pop_array_string" => StackEffect { pops: 2, pushes: 0 },
        "pop_array_int_leave_value_on_stack" => StackEffect { pops: 2, pushes: 1 },

        // Binary arithmetic: pops 2, pushes 1 (opcode is `modulo`, not `mod`)
        "add" | "sub" | "multiply" | "divide" | "modulo" => StackEffect { pops: 2, pushes: 1 },

        // Comparison/logical: pops 2, pushes 1 (result int)
        "compare" | "and" | "or" => StackEffect { pops: 2, pushes: 1 },

        // Unary: pops 1, pushes 1
        "lowercase" | "uppercase" | "length" | "neg" => StackEffect { pops: 1, pushes: 1 },

        // String join: pops N, pushes 1
        "join_string" => {
            if let OperandNode::Count(n) = operand {
                StackEffect {
                    pops: *n,
                    pushes: 1,
                }
            } else {
                StackEffect { pops: 0, pushes: 1 }
            }
        }

        // Array define: pops 0, pushes 0
        "define_array" => StackEffect { pops: 0, pushes: 0 },

        // Control flow
        "branch" => StackEffect { pops: 0, pushes: 0 },
        "branch_not" | "branch_if_true" | "branch_if_false" => StackEffect { pops: 1, pushes: 0 },
        "branch_equals"
        | "branch_less_than"
        | "branch_greater_than"
        | "branch_less_than_or_equals"
        | "branch_greater_than_or_equals"
        | "long_branch_equals"
        | "long_branch_less_than"
        | "long_branch_greater_than"
        | "long_branch_less_than_or_equals"
        | "long_branch_greater_than_or_equals" => StackEffect { pops: 2, pushes: 0 },
        "switch" => StackEffect { pops: 1, pushes: 0 },
        "return" => StackEffect { pops: 0, pushes: 0 },

        "gosub_with_params" => {
            if let OperandNode::Script(id) = operand
                && let Some((_target, signature)) =
                    super::resolve_call_target_signature(script_catalog, script_signatures, *id)
            {
                StackEffect {
                    pops: signature.total_args(),
                    pushes: usize::from(signature.return_type != "void"),
                }
            } else {
                StackEffect { pops: 0, pushes: 0 }
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
        | "player_group_get_displayname"
        | "activeclanchannel_find_affined"
        | "activeclanchannel_find_listened"
        | "activeclanchannel_getclanname"
        | "activeclanchannel_getrankkick"
        | "activeclanchannel_getranktalk"
        | "activeclanchannel_getusercount"
        | "activeclansettings_find_affined"
        | "activeclansettings_find_listened"
        | "activeclansettings_getallowunaffined"
        | "activeclansettings_getaffinedcount"
        | "activeclansettings_getclanname"
        | "activeclansettings_getbannedcount"
        | "activeclansettings_getcoinshare"
        | "activeclansettings_getcurrentowner_slot"
        | "activeclansettings_getrankkick"
        | "activeclansettings_getranklootshare"
        | "activeclansettings_getranktalk"
        | "activeclansettings_getreplacementowner_slot"
        | "clanprofile_find" => StackEffect { pops: 0, pushes: 1 },
        "player_group_member_get_rank"
        | "player_group_member_get_team"
        | "player_group_member_get_last_seen_node_id"
        | "player_group_member_get_status"
        | "player_group_member_is_online"
        | "player_group_member_is_member"
        | "player_group_member_get_displayname"
        | "player_group_member_is_owner"
        | "player_group_banned_get_displayname"
        | "activeclanchannel_getuserdisplayname"
        | "activeclanchannel_getuserrank"
        | "activeclanchannel_getuserworld"
        | "activeclanchannel_getsorteduserslot"
        | "activeclansettings_getaffineddisplayname"
        | "activeclansettings_getaffinedrank"
        | "activeclansettings_getaffinedmuted"
        | "activeclansettings_getbanneddisplayname"
        | "activeclansettings_getaffinedjoinruneday"
        | "activeclansettings_getsortedaffinedslot" => StackEffect { pops: 1, pushes: 1 },
        "player_group_member_get_join_xp"
        | "player_group_member_get_same_world_var"
        | "activeclansettings_getaffinedextrainfo" => StackEffect { pops: 3, pushes: 1 },
        "activeclanchannel_getuserslot" | "activeclansettings_getaffinedslot" => {
            StackEffect { pops: 1, pushes: 1 }
        }
        "activeclanchannel_kickuser" | "affinedclansettings_addbanned_fromchannel" => {
            StackEffect { pops: 1, pushes: 0 }
        }
        "affinedclansettings_setmuted_fromchannel" => StackEffect { pops: 2, pushes: 0 },
        "login_last_transfer_reply" | "autosetup_dosetup" | "autosetup_dosetupstatus" => {
            StackEffect {
                pops: 0,
                pushes: if cmd == "login_last_transfer_reply" {
                    3
                } else {
                    2
                },
            }
        }
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
        | "create_under13" => StackEffect { pops: 0, pushes: 1 },
        "create_suggest_name_reply" => StackEffect { pops: 0, pushes: 2 },
        "create_get_email" | "sso_displayname" => StackEffect { pops: 0, pushes: 1 },
        "get_currentcursor" | "get_mousex" | "get_mousey" => StackEffect { pops: 0, pushes: 1 },
        "get_mousebuttons" => StackEffect { pops: 0, pushes: 3 },
        "get_active_minimenu_entry" | "get_second_minimenu_entry" => {
            StackEffect { pops: 0, pushes: 4 }
        }
        "get_minimenu_length" => StackEffect { pops: 0, pushes: 2 },
        "get_minimenu_target" => StackEffect { pops: 0, pushes: 3 },
        "worldlist_start" | "worldlist_next" => StackEffect { pops: 0, pushes: 8 },
        "worldlist_fetch" | "worldlist_specific_thisworld" => StackEffect { pops: 0, pushes: 1 },
        "worldlist_specific" => StackEffect { pops: 1, pushes: 7 },
        "worldlist_switch" => StackEffect { pops: 2, pushes: 1 },
        "worldlist_sort" => StackEffect { pops: 4, pushes: 0 },
        "worldlist_autoworld" => StackEffect { pops: 0, pushes: 0 },
        "worldlist_pingworlds" => StackEffect { pops: 1, pushes: 0 },
        "pushCanvasSize" | "viewport_geteffectivesize" => StackEffect { pops: 0, pushes: 2 },
        "pushZeroInsets" => StackEffect { pops: 0, pushes: 4 },
        "pushFontMetrics" => StackEffect { pops: 1, pushes: 5 },
        "viewport_getzoom" | "viewport_getfov" | "fullscreen_getmode" => StackEffect {
            pops: usize::from(cmd == "fullscreen_getmode"),
            pushes: 2,
        },
        "fullscreen_enter" => StackEffect { pops: 2, pushes: 1 },
        "fullscreen_modecount" => StackEffect { pops: 0, pushes: 1 },
        "window_getinsets" => StackEffect { pops: 0, pushes: 4 },
        "targetmode_active" | "if_get_gamescreen" | "interface_getpickingradius" => {
            StackEffect { pops: 0, pushes: 1 }
        }
        "if_hassub" | "if_getnextsubid" => StackEffect { pops: 1, pushes: 1 },
        "if_hassubmodal" | "if_hassuboverlay" => StackEffect { pops: 2, pushes: 1 },
        "setup_messagebox" => StackEffect {
            pops: 11,
            pushes: 0,
        },
        "formatminimenu" => StackEffect {
            pops: 12,
            pushes: 0,
        },
        "minimenuopen" => StackEffect { pops: 2, pushes: 1 },
        "setsubmenuminlength" => StackEffect { pops: 1, pushes: 0 },
        "if_set_gamescreen_enabled" | "interface_setpickingradius" => {
            StackEffect { pops: 1, pushes: 0 }
        }
        "if_opensubclient" => StackEffect { pops: 2, pushes: 0 },
        "if_closesubclient" => StackEffect { pops: 1, pushes: 0 },
        "opplayer" => StackEffect { pops: 2, pushes: 0 },
        "opplayert" => StackEffect { pops: 1, pushes: 0 },
        "defaultminimenu"
        | "minimenu_close"
        | "if_close"
        | "force_interface_drag"
        | "cancel_interface_drag"
        | "targetmode_cancel" => StackEffect { pops: 0, pushes: 0 },
        "login_accountappeal"
        | "shop_getindexforcategoryid"
        | "shop_getcategoryid"
        | "shop_getproductcount"
        | "shop_getcategorydescription"
        | "shop_isproductavailable"
        | "shop_isproductrecommended"
        | "shop_getindexforcategoryname" => StackEffect { pops: 1, pushes: 1 },
        "lobby_entergamereply"
        | "lobby_enterlobbyreply"
        | "shop_purchaseitemstatus"
        | "sso_available" => StackEffect { pops: 0, pushes: 1 },
        "shop_getproductdetails" => StackEffect { pops: 2, pushes: 9 },
        "notifications_sendlocal" => StackEffect { pops: 4, pushes: 1 },
        "notifications_sendgroupedlocal" => StackEffect { pops: 6, pushes: 1 },
        "login_request_social_network" => StackEffect { pops: 3, pushes: 0 },
        "lobby_entergame" => StackEffect { pops: 2, pushes: 0 },
        "lobby_enterlobby" => StackEffect { pops: 4, pushes: 0 },
        "lobby_enterlobby_sso" => StackEffect { pops: 2, pushes: 0 },
        "create_createrequest" => StackEffect { pops: 5, pushes: 0 },
        "db_listall" => StackEffect { pops: 1, pushes: 1 },
        "db_find" => StackEffect { pops: 2, pushes: 0 },
        "db_find_with_count" => StackEffect { pops: 2, pushes: 1 },
        "db_findnext" => StackEffect { pops: 0, pushes: 1 },
        // DB field arity depends on column schema. Recover one best-effort
        // value here rather than dropping stack behavior entirely.
        "db_getfield" => StackEffect { pops: 3, pushes: 1 },
        "db_getfieldcount" => StackEffect { pops: 2, pushes: 1 },
        "db_find_refine" => StackEffect { pops: 2, pushes: 1 },
        "db_find_get" | "db_getrowtable" => StackEffect { pops: 1, pushes: 1 },
        "create_availablerequest"
        | "create_name_availablerequest"
        | "create_step_reached"
        | "shop_open"
        | "notifications_cancellocal"
        | "shop_purchaseitem"
        | "marketing_sendevent" => StackEffect { pops: 1, pushes: 0 },
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
        | "create_setunder13" => StackEffect { pops: 0, pushes: 0 },

        // CC ops: various stack effects
        "cc_settext"
        | "cc_setgraphic"
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
        | "cc_setstylesheet" => StackEffect { pops: 1, pushes: 0 },
        "cc_setscrollpos" | "cc_setscrollsize" | "cc_setaspect" | "cc_setmodelorigin"
        | "cc_setparam" | "cc_setparam_int" | "cc_setparam_string" => {
            StackEffect { pops: 2, pushes: 0 }
        }
        "cc_settextalign" | "cc_setrecol" | "cc_setretex" => StackEffect { pops: 3, pushes: 0 },
        "cc_setposition" | "cc_setsize" | "cc_setmodeltint" => StackEffect { pops: 4, pushes: 0 },
        "cc_setmodelangle" => StackEffect { pops: 6, pushes: 0 },
        "cc_setmodellighting" => StackEffect {
            pops: 10,
            pushes: 0,
        },
        "cc_delete"
        | "cc_sendtofront"
        | "cc_sendtoback"
        | "cc_resetmodellighting"
        | "cc_clearops"
        | "cc_callonresize" => StackEffect { pops: 0, pushes: 0 },
        "cc_deleteall" | "if_sendtofront" | "if_sendtoback" => StackEffect { pops: 1, pushes: 0 },
        "cc_find" => StackEffect { pops: 2, pushes: 1 },
        "if_find" => StackEffect { pops: 1, pushes: 1 },
        "if_gettext" => StackEffect { pops: 1, pushes: 1 },
        "if_settext"
        | "if_sethide"
        | "if_setcolour"
        | "if_setfill"
        | "if_settrans"
        | "if_setlinewid"
        | "if_setgraphic"
        | "if_setmodel"
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
        | "if_resetmodellighting"
        | "if_setonclick"
        | "if_setonvartransmit"
        | "if_setonstocktransmit"
        | "if_setoninvtransmit" => StackEffect { pops: 2, pushes: 0 },
        "if_setscrollpos" | "if_setscrollsize" | "if_setaspect" | "if_setmodelorigin"
        | "if_setparam_int" | "if_setparam_string" => StackEffect { pops: 3, pushes: 0 },
        "if_settextalign" | "if_setrecol" | "if_setretex" => StackEffect { pops: 4, pushes: 0 },
        "if_setposition" | "if_setsize" | "if_setmodeltint" => StackEffect { pops: 5, pushes: 0 },
        "if_setmodelangle" => StackEffect { pops: 7, pushes: 0 },
        "if_setmodellighting" => StackEffect {
            pops: 11,
            pushes: 0,
        },

        // Misc known ops
        "cc_create" => StackEffect { pops: 3, pushes: 0 },
        "baseidkit" | "basecolour" | "setgender" | "setobj" => StackEffect { pops: 0, pushes: 0 },
        "quickchat_dynamic_command_add" => StackEffect { pops: 2, pushes: 1 },
        "cc_param" => StackEffect { pops: 1, pushes: 1 },
        "oc_param" | "nc_param" | "lc_param" | "struct_param" | "seq_param" | "mec_param"
        | "quest_param" | "enum_string" => StackEffect { pops: 2, pushes: 1 },
        "enum_hasoutput" | "enum_getreversecount" => StackEffect { pops: 3, pushes: 1 },
        "enum_hasoutput_string" | "enum_getreversecount_string" => {
            StackEffect { pops: 2, pushes: 1 }
        }
        "enum" | "_enum" => StackEffect { pops: 4, pushes: 1 },
        "enum_getoutputcount" => StackEffect { pops: 1, pushes: 1 },
        "enum_getreverseindex" => StackEffect { pops: 5, pushes: 1 },
        "enum_getreverseindex_string" => StackEffect { pops: 4, pushes: 1 },

        // ── Component getters and value ops (effects extracted from the client
        // ScriptRunner; counts are int+string-stack totals since recovery uses a
        // single unified expression stack). These push a value the default
        // CC/IF categorisation modelled as a void pop, stranding the result and
        // forcing a residual pop(). Only single-push getters are listed; the
        // few that push two values (dimensions/invcount/getop/getopbase/
        // nextsubid) need multi-value recovery and are intentionally left to the
        // default until that lands. ──
        // cc_* getters operate on the current component (no argument).
        "cc_get2dangle" | "cc_getcolour" | "cc_getfontgraphic" | "cc_getfontmetrics"
        | "cc_getgraphic" | "cc_getheight" | "cc_gethide" | "cc_getid" | "cc_getinvobject"
        | "cc_getlayer" | "cc_getmodel" | "cc_getmodelangle_x" | "cc_getmodelangle_y"
        | "cc_getmodelangle_z" | "cc_getmodelxof" | "cc_getmodelyof" | "cc_getmodelzoom"
        | "cc_getparentlayer" | "cc_getscrollheight" | "cc_getscrollwidth" | "cc_getscrollx"
        | "cc_getscrolly" | "cc_gettargetmask" | "cc_gettext" | "cc_gettrans" | "cc_getwidth"
        | "cc_getx" | "cc_gety" | "if_gettop" | "clientclock" => StackEffect { pops: 0, pushes: 1 },
        // if_* getters take an explicit component id.
        "if_get2dangle" | "if_getcolour" | "if_getfontgraphic" | "if_getfontmetrics"
        | "if_getgraphic" | "if_getheight" | "if_gethide" | "if_getinvobject" | "if_getlayer"
        | "if_getmodel" | "if_getmodelangle_x" | "if_getmodelangle_y" | "if_getmodelangle_z"
        | "if_getmodelxof" | "if_getmodelyof" | "if_getmodelzoom" | "if_getparentlayer"
        | "if_getscrollheight" | "if_getscrollwidth" | "if_getscrollx" | "if_getscrolly"
        | "if_gettargetmask" | "if_gettrans" | "if_getwidth" | "if_getx" | "if_gety"
        | "tostring" | "oc_name" | "string_length" => StackEffect { pops: 1, pushes: 1 },
        "max" | "min" | "testbit" | "append" | "tostring_localised" => {
            StackEffect { pops: 2, pushes: 1 }
        }
        "scale" => StackEffect { pops: 3, pushes: 1 },
        "movecoord" => StackEffect { pops: 4, pushes: 1 },

        // The explicit arms above are hand-verified and win; for everything else
        // consult the client-extracted opcode table (the long tail of getters,
        // config lookups and value ops) before the coarse categorisation
        // default. A wrong table effect only leaves its scripts gate-blocked,
        // never miscompiles.
        _ => {
            if let Some(&(pops, pushes)) = opcode_stack_effects().get(cmd) {
                StackEffect { pops, pushes }
            } else {
                match categorize(cmd) {
                    OpcodeCategory::Push => StackEffect { pops: 0, pushes: 1 },
                    OpcodeCategory::Pop
                    | OpcodeCategory::Branch
                    | OpcodeCategory::CC
                    | OpcodeCategory::IF => StackEffect { pops: 1, pushes: 0 },
                    OpcodeCategory::Other => StackEffect { pops: 0, pushes: 0 },
                }
            }
        }
    }
}

/// Stack effect of an opcode: `(pops, pushes, pushed_type)`. `pushed_type` is
/// the stack the produced value lands on — used by the lowerer to recover a
/// generic command call's result kind.
#[derive(Clone, Copy)]
pub struct OpcodeStackEffect {
    pub pops: usize,
    pub pushes: usize,
    pub pushed_type: PushedType,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PushedType {
    Int,
    Obj,
    Long,
    None,
    Multi,
}

/// The client-extracted opcode stack-effect table (see
/// `scripts/extract-stack-effects.py` and `data/stack-effects.txt`). Keyed by
/// command name; build independent.
pub fn opcode_stack_effect(command: &str) -> Option<OpcodeStackEffect> {
    opcode_stack_effect_full().get(command).copied()
}

fn opcode_stack_effects() -> &'static std::collections::HashMap<&'static str, (usize, usize)> {
    static COUNTS: std::sync::OnceLock<std::collections::HashMap<&'static str, (usize, usize)>> =
        std::sync::OnceLock::new();
    COUNTS.get_or_init(|| {
        opcode_stack_effect_full()
            .iter()
            .map(|(&name, e)| (name, (e.pops, e.pushes)))
            .collect()
    })
}

fn opcode_stack_effect_full() -> &'static std::collections::HashMap<&'static str, OpcodeStackEffect>
{
    static TABLE: std::sync::OnceLock<std::collections::HashMap<&'static str, OpcodeStackEffect>> =
        std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut map = std::collections::HashMap::new();
        for line in include_str!("../../data/stack-effects.txt").lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut fields = line.split_whitespace();
            let (Some(name), Some(pops), Some(pushes), Some(kind)) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let (Ok(pops), Ok(pushes)) = (pops.parse::<usize>(), pushes.parse::<usize>()) else {
                continue;
            };
            let pushed_type = match kind {
                "int" => PushedType::Int,
                "obj" => PushedType::Obj,
                "long" => PushedType::Long,
                "multi" => PushedType::Multi,
                _ => PushedType::None,
            };
            map.insert(
                name,
                OpcodeStackEffect {
                    pops,
                    pushes,
                    pushed_type,
                },
            );
        }
        map
    })
}

#[derive(Debug, Clone)]
pub enum RecoveredStmt {
    Expression(Expression),
    Assignment {
        target: String,
        value: Expression,
        var_type: String,
    },
    Goto(usize),
    Branch {
        condition: Expression,
        target: usize,
        negated: bool,
    },
    BranchBinary {
        op: BinaryOp,
        left: Expression,
        right: Expression,
        target: usize,
    },
    Switch {
        discriminant: Expression,
        cases: Vec<(i32, usize)>,
    },
    Return(Option<Expression>),
    Comment(String),
}

/// Scan recovered statements for `return` ops without building CFG or
/// structured output. This keeps signature inference cheap even for
/// pathological control-flow graphs.
pub fn detect_return_type_from_recovered(stmts: &[Option<RecoveredStmt>]) -> &'static str {
    let mut has_value_return = false;
    let mut has_void_return = false;

    for stmt in stmts.iter().flatten() {
        if let RecoveredStmt::Return(value) = stmt {
            if value.is_some() {
                has_value_return = true;
            } else {
                has_void_return = true;
            }
        }
    }

    match (has_value_return, has_void_return) {
        (true, false) => "number",
        (false, true) => "void",
        (true, true) => "number | void",
        (false, false) => "void",
    }
}

pub struct ExprRecovery<'a, S: std::hash::BuildHasher = std::collections::hash_map::RandomState> {
    instructions: &'a [InstructionNode],
    stack: Vec<Expression>,
    locals: std::collections::HashMap<String, Expression>,
    var_names: &'a std::collections::HashMap<(VarDomain, u16), String>,
    /// Maps enum key values to qualified names (e.g. 0 → "`Enum_1234.ATTACK`").
    enum_value_names: &'a std::collections::HashMap<i32, String, S>,
    script_catalog: &'a super::ScriptCatalog,
    script_signatures: &'a std::collections::HashMap<super::ScriptId, super::ScriptSignature>,
}

impl<'a, S: std::hash::BuildHasher> ExprRecovery<'a, S> {
    pub fn new(
        instructions: &'a [InstructionNode],
        var_names: &'a std::collections::HashMap<(VarDomain, u16), String>,
        _component_names: &'a std::collections::HashMap<u32, String, S>,
        enum_value_names: &'a std::collections::HashMap<i32, String, S>,
        script_catalog: &'a super::ScriptCatalog,
        script_signatures: &'a std::collections::HashMap<super::ScriptId, super::ScriptSignature>,
    ) -> Self {
        Self {
            instructions,
            stack: Vec::new(),
            locals: std::collections::HashMap::new(),
            var_names,
            enum_value_names,
            script_catalog,
            script_signatures,
        }
    }

    /// Process all instructions and return recovered statements.
    /// The result may have fewer entries than instructions (push/pop are
    /// folded into expressions).
    pub fn recover(mut self) -> Vec<Option<RecoveredStmt>> {
        let len = self.instructions.len();
        let mut stmts: Vec<Option<RecoveredStmt>> = vec![None; len];

        for (i, stmt_slot) in stmts.iter_mut().enumerate().take(len) {
            let instr = &self.instructions[i];
            let effect = stack_effect(
                &instr.command,
                &instr.operand,
                self.script_catalog,
                self.script_signatures,
            );
            *stmt_slot = self.process_instruction(instr, &effect);
        }

        stmts
    }

    // Process is a large match with many `if let` arms that need early
    // return to avoid nested else branches. Converting to expression-
    // based returns would obscure the opcode dispatch pattern.
    #[allow(clippy::needless_return)]
    fn process_instruction(
        &mut self,
        instr: &InstructionNode,
        effect: &StackEffect,
    ) -> Option<RecoveredStmt> {
        let cmd = instr.command.as_str();
        let op = &instr.operand;

        match cmd {
            // ── Push operations: build an expression and push onto stack ──
            "push_constant_int" => {
                if let OperandNode::Int(v) = op {
                    self.stack
                        .push(Expression::NumberLiteral(NumberLiteral { value: *v }));
                }
                None
            }
            "push_long_constant" => {
                if let OperandNode::Long(v) = op {
                    self.stack
                        .push(Expression::BigIntLiteral(super::ast::BigIntLiteral {
                            value: *v,
                        }));
                }
                None
            }
            "push_constant_string" => {
                if let OperandNode::String(s) = op {
                    if let Some(callback) = parse_callback_literal(s) {
                        self.stack.push(Expression::CallbackLiteral(callback));
                    } else {
                        self.stack.push(Expression::StringLiteral(StringLiteral {
                            value: s.clone(),
                        }));
                    }
                } else if let OperandNode::Int(v) = op {
                    // Only resolve non-negative values as enum keys.
                    // Negative values (e.g. -1) are sentinel/not-found markers.
                    if *v >= 0 {
                        if let Some(qualified) = self.enum_value_names.get(v) {
                            if let Some(dot) = qualified.find('.') {
                                let obj = &qualified[..dot];
                                let prop = &qualified[dot + 1..];
                                self.stack.push(Expression::PropertyAccess(
                                    super::ast::PropertyAccess {
                                        object: Box::new(Expression::Identifier(Identifier {
                                            name: obj.to_string(),
                                        })),
                                        property: prop.to_string(),
                                    },
                                ));
                            } else {
                                self.stack
                                    .push(Expression::NumberLiteral(NumberLiteral { value: *v }));
                            }
                        } else {
                            self.stack
                                .push(Expression::NumberLiteral(NumberLiteral { value: *v }));
                        }
                    } else {
                        self.stack
                            .push(Expression::NumberLiteral(NumberLiteral { value: *v }));
                    }
                }
                None
            }
            "push_var"
            | "push_varc_int"
            | "push_varc_string"
            | "push_varclan"
            | "push_varclan_long"
            | "push_varclan_string"
            | "push_varclansetting"
            | "push_varclansetting_long"
            | "push_varclansetting_string" => {
                if let OperandNode::VarRef(vr) = op {
                    let name = vr.name.clone().unwrap_or_else(|| {
                        format!("VARS.get({} * 1000000 + {})", u64::from(vr.domain), vr.id)
                    });
                    self.stack.push(Expression::Identifier(Identifier { name }));
                }
                None
            }
            "push_varbit" | "push_varclanbit" | "push_varclansettingbit" => {
                if let OperandNode::VarBitRef(vbr) = op {
                    let name = vbr
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("VARBITS.get({})", vbr.id));
                    self.stack.push(Expression::Identifier(Identifier { name }));
                }
                None
            }
            "pop_varbit" => {
                if let OperandNode::VarBitRef(vbr) = op {
                    let value = self.pop_or_unknown();
                    let name = vbr
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("VARBITS.get({})", vbr.id));
                    return Some(RecoveredStmt::Assignment {
                        target: name,
                        value,
                        var_type: "number".to_string(),
                    });
                }
                None
            }
            "push_int_local" | "push_string_local" | "push_long_local" => {
                if let OperandNode::Local(idx) = op {
                    let (prefix, _) = local_type(cmd);
                    let name = format!("{prefix}_{idx}");
                    self.stack.push(Expression::Identifier(Identifier { name }));
                }
                None
            }
            "push_array_int" | "push_array_string" => {
                if let OperandNode::Array(id) = op {
                    let idx = self.pop_expr()?;
                    let arr = Expression::Identifier(Identifier {
                        name: format!("array_{id}"),
                    });
                    self.stack
                        .push(Expression::ArrayAccess(super::ast::ArrayAccess {
                            array: Box::new(arr),
                            index: Box::new(idx),
                        }));
                }
                None
            }
            "push_array_int_leave_index_on_stack" => {
                if let OperandNode::Array(id) = op {
                    let idx = self.pop_expr()?;
                    let arr = Expression::Identifier(Identifier {
                        name: format!("array_{id}"),
                    });
                    let access = Expression::ArrayAccess(super::ast::ArrayAccess {
                        array: Box::new(arr),
                        index: Box::new(idx.clone()),
                    });
                    self.stack.push(idx);
                    self.stack.push(access);
                }
                None
            }
            "push_array_int_and_index" => {
                if let OperandNode::Array(id) = op {
                    let idx = self.pop_expr()?;
                    let arr = Expression::Identifier(Identifier {
                        name: format!("array_{id}"),
                    });
                    let access = Expression::ArrayAccess(super::ast::ArrayAccess {
                        array: Box::new(arr),
                        index: Box::new(idx.clone()),
                    });
                    self.stack.push(access);
                    self.stack.push(idx);
                }
                None
            }
            // ── Pop operations: pop from stack, produce assignment or discard ──
            "pop_int_local" | "pop_string_local" | "pop_long_local" => {
                if let OperandNode::Local(idx) = op {
                    let value = self.pop_expr().unwrap_or_else(|| {
                        Expression::Call(CallExpr {
                            callee: Box::new(Expression::Identifier(Identifier {
                                name: "pop".to_string(),
                            })),
                            arguments: vec![],
                        })
                    });
                    let (prefix, var_type) = local_type(cmd);
                    let name = format!("{prefix}_{idx}");
                    self.locals.insert(name.clone(), value.clone());
                    return Some(RecoveredStmt::Assignment {
                        target: name,
                        value,
                        var_type: var_type.to_string(),
                    });
                }
                None
            }
            "pop_var" => {
                if let OperandNode::VarRef(vr) = op {
                    let value = self.pop_or_unknown();
                    let name = vr.name.clone().unwrap_or_else(|| {
                        format!("VARS.get({} * 1000000 + {})", u64::from(vr.domain), vr.id)
                    });
                    return Some(RecoveredStmt::Assignment {
                        target: name,
                        value,
                        var_type: "number".to_string(),
                    });
                }
                None
            }
            "pop_int_discard" | "pop_string_discard" | "pop_long_discard" => {
                self.stack.pop();
                None
            }
            "pop_array_int" | "pop_array_string" => {
                if let OperandNode::Array(id) = op {
                    let value = self.stack.pop().unwrap_or_else(|| {
                        Expression::Call(CallExpr {
                            callee: Box::new(Expression::Identifier(Identifier {
                                name: "pop".to_string(),
                            })),
                            arguments: vec![],
                        })
                    });
                    let idx = self
                        .pop_expr()
                        .unwrap_or(Expression::NumberLiteral(NumberLiteral { value: 0 }));
                    let idx_str = expr_str(&idx);
                    return Some(RecoveredStmt::Assignment {
                        target: format!("array_{id}[{idx_str}]"),
                        value,
                        var_type: "number".to_string(),
                    });
                }
                None
            }
            "pop_array_int_leave_value_on_stack" => {
                if let OperandNode::Array(id) = op {
                    let value = self.stack.pop().unwrap_or_else(|| {
                        Expression::Call(CallExpr {
                            callee: Box::new(Expression::Identifier(Identifier {
                                name: "pop".to_string(),
                            })),
                            arguments: vec![],
                        })
                    });
                    let idx = self
                        .pop_expr()
                        .unwrap_or(Expression::NumberLiteral(NumberLiteral { value: 0 }));
                    let idx_str = expr_str(&idx);
                    self.stack.push(value.clone());
                    return Some(RecoveredStmt::Assignment {
                        target: format!("array_{id}[{idx_str}]"),
                        value,
                        var_type: "number".to_string(),
                    });
                }
                None
            }

            // ── Binary arithmetic: pop 2, build expression, push result ──
            "add" | "sub" | "multiply" | "divide" | "modulo" => {
                let right = self.pop_or_unknown();
                let left = self.pop_or_unknown();
                let op = match cmd {
                    "add" => BinaryOp::Add,
                    "sub" => BinaryOp::Sub,
                    "multiply" => BinaryOp::Mul,
                    "divide" => BinaryOp::Div,
                    "modulo" => BinaryOp::Mod,
                    _ => unreachable!(),
                };
                let expr = Expression::BinaryOperation(super::ast::BinaryOperation {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                });
                self.stack.push(expr);
                None
            }
            "compare" => {
                let right = self.pop_or_unknown();
                let left = self.pop_or_unknown();
                let expr = Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "compare".to_string(),
                    })),
                    arguments: vec![left, right],
                });
                self.stack.push(expr);
                None
            }
            "and" => {
                let right = self.pop_or_unknown();
                let left = self.pop_or_unknown();
                self.stack
                    .push(Expression::BinaryOperation(super::ast::BinaryOperation {
                        op: BinaryOp::And,
                        left: Box::new(left),
                        right: Box::new(right),
                    }));
                None
            }
            "or" => {
                let right = self.pop_or_unknown();
                let left = self.pop_or_unknown();
                self.stack
                    .push(Expression::BinaryOperation(super::ast::BinaryOperation {
                        op: BinaryOp::Or,
                        left: Box::new(left),
                        right: Box::new(right),
                    }));
                None
            }

            // ── Unary ops ──
            "lowercase" => {
                let arg = self.pop_or_unknown();
                let expr = Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "lowercase".to_string(),
                    })),
                    arguments: vec![arg],
                });
                self.stack.push(expr);
                None
            }
            "uppercase" => {
                let arg = self.pop_or_unknown();
                let expr = Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "uppercase".to_string(),
                    })),
                    arguments: vec![arg],
                });
                self.stack.push(expr);
                None
            }
            "length" => {
                let arg = self.pop_or_unknown();
                let expr = Expression::PropertyAccess(super::ast::PropertyAccess {
                    object: Box::new(arg),
                    property: "length".to_string(),
                });
                self.stack.push(expr);
                None
            }
            "neg" => {
                let arg = self.pop_or_unknown();
                let expr = Expression::UnaryOperation(super::ast::UnaryOperation {
                    op: super::ast::UnaryOp::Neg,
                    operand: Box::new(arg),
                });
                self.stack.push(expr);
                None
            }

            // ── String join ──
            "join_string" => {
                if let OperandNode::Count(n) = op {
                    let Some(part_count) = Some(*n)
                        .filter(|count| *count <= self.stack.len())
                        .filter(|count| *count <= MAX_JOIN_STRING_PARTS)
                    else {
                        self.stack.push(Expression::Call(CallExpr {
                            callee: Box::new(Expression::Identifier(Identifier {
                                name: "concat".to_string(),
                            })),
                            arguments: Vec::new(),
                        }));
                        return Some(RecoveredStmt::Comment(format!(
                            "invalid join_string count {n}"
                        )));
                    };
                    let mut parts: Vec<Expression> =
                        (0..part_count).map(|_| self.pop_or_unknown()).collect();
                    parts.reverse();
                    let expr = Expression::Call(CallExpr {
                        callee: Box::new(Expression::Identifier(Identifier {
                            name: "concat".to_string(),
                        })),
                        arguments: parts,
                    });
                    self.stack.push(expr);
                }
                None
            }

            // ── Control flow ──
            "branch" => {
                if let OperandNode::Branch(target) = op {
                    self.stack.clear();
                    return Some(RecoveredStmt::Goto(*target));
                }
                None
            }
            "branch_not" => {
                if let OperandNode::Branch(target) = op {
                    let condition = self.pop_or_unknown();
                    self.stack.clear();
                    return Some(RecoveredStmt::Branch {
                        condition,
                        target: *target,
                        negated: true,
                    });
                }
                None
            }
            "branch_if_true" => {
                if let OperandNode::Branch(target) = op {
                    let condition = self.pop_or_unknown();
                    self.stack.clear();
                    return Some(RecoveredStmt::Branch {
                        condition,
                        target: *target,
                        negated: false,
                    });
                }
                None
            }
            "branch_if_false" => {
                if let OperandNode::Branch(target) = op {
                    let condition = self.pop_or_unknown();
                    self.stack.clear();
                    return Some(RecoveredStmt::Branch {
                        condition,
                        target: *target,
                        negated: true,
                    });
                }
                None
            }
            "branch_equals" => {
                if let OperandNode::Branch(target) = op {
                    let right = self.pop_or_unknown();
                    let left = self.pop_or_unknown();
                    self.stack.clear();
                    return Some(RecoveredStmt::BranchBinary {
                        op: BinaryOp::Eq,
                        left,
                        right,
                        target: *target,
                    });
                }
                None
            }
            "branch_less_than" => {
                if let OperandNode::Branch(target) = op {
                    let right = self.pop_or_unknown();
                    let left = self.pop_or_unknown();
                    self.stack.clear();
                    return Some(RecoveredStmt::BranchBinary {
                        op: BinaryOp::Lt,
                        left,
                        right,
                        target: *target,
                    });
                }
                None
            }
            "branch_greater_than" => {
                if let OperandNode::Branch(target) = op {
                    let right = self.pop_or_unknown();
                    let left = self.pop_or_unknown();
                    self.stack.clear();
                    return Some(RecoveredStmt::BranchBinary {
                        op: BinaryOp::Gt,
                        left,
                        right,
                        target: *target,
                    });
                }
                None
            }
            "branch_less_than_or_equals" => {
                if let OperandNode::Branch(target) = op {
                    let right = self.pop_or_unknown();
                    let left = self.pop_or_unknown();
                    self.stack.clear();
                    return Some(RecoveredStmt::BranchBinary {
                        op: BinaryOp::Le,
                        left,
                        right,
                        target: *target,
                    });
                }
                None
            }
            "branch_greater_than_or_equals" => {
                if let OperandNode::Branch(target) = op {
                    let right = self.pop_or_unknown();
                    let left = self.pop_or_unknown();
                    self.stack.clear();
                    return Some(RecoveredStmt::BranchBinary {
                        op: BinaryOp::Ge,
                        left,
                        right,
                        target: *target,
                    });
                }
                None
            }
            "switch" => {
                if let OperandNode::Switch(cases) = op {
                    let discriminant = self.pop_or_unknown();
                    let case_pairs: Vec<(i32, usize)> =
                        cases.iter().map(|c| (c.value, c.target)).collect();
                    self.stack.clear();
                    return Some(RecoveredStmt::Switch {
                        discriminant,
                        cases: case_pairs,
                    });
                }
                None
            }
            "return" => {
                let val = self.stack.pop();
                return Some(RecoveredStmt::Return(val));
            }

            // ── Script call ──
            "gosub_with_params" => {
                if let OperandNode::Script(id) = op {
                    let Some((target, signature)) = super::resolve_call_target_signature(
                        self.script_catalog,
                        self.script_signatures,
                        *id,
                    ) else {
                        return Some(RecoveredStmt::Comment(format!(
                            "unresolved gosub_with_params target script {id}"
                        )));
                    };
                    let total_args = signature.total_args();
                    let mut args: Vec<Expression> =
                        (0..total_args).map(|_| self.pop_or_unknown()).collect();
                    args.reverse();
                    let expr = Expression::Call(CallExpr {
                        callee: Box::new(Expression::Identifier(Identifier {
                            name: target.export_name.clone(),
                        })),
                        arguments: args,
                    });
                    if signature.return_type == "void" {
                        return Some(RecoveredStmt::Expression(expr));
                    }
                    self.stack.push(expr);
                }
                None
            }
            "quickchat_dynamic_command_add" => {
                let right = self.pop_or_unknown();
                let left = self.pop_or_unknown();
                self.stack
                    .push(Expression::BinaryOperation(super::ast::BinaryOperation {
                        op: BinaryOp::Sub,
                        left: Box::new(left),
                        right: Box::new(right),
                    }));
                None
            }
            "get_mousebuttons" => {
                self.push_named_command_results(cmd, &["primary", "middle", "secondary"]);
                None
            }
            "get_active_minimenu_entry" | "get_second_minimenu_entry" => {
                self.push_named_command_results(
                    cmd,
                    &["entityType", "op", "opBase", "questIconSuffix"],
                );
                None
            }
            "get_minimenu_length" => {
                self.push_indexed_command_results(cmd, 2);
                None
            }
            "get_minimenu_target" => {
                self.push_indexed_command_results(cmd, 3);
                None
            }
            "worldlist_start" | "worldlist_next" => {
                self.push_named_command_results(
                    cmd,
                    &[
                        "id",
                        "flags",
                        "activity",
                        "countryId",
                        "countryName",
                        "players",
                        "ping",
                        "host",
                    ],
                );
                None
            }
            "worldlist_specific" => {
                let world = self.pop_or_unknown();
                self.push_named_call_results(
                    cmd,
                    vec![world],
                    &[
                        "flags",
                        "activity",
                        "countryId",
                        "countryName",
                        "players",
                        "ping",
                        "host",
                    ],
                );
                None
            }
            "pushCanvasSize" | "viewport_geteffectivesize" => {
                self.push_named_command_results(cmd, &["width", "height"]);
                None
            }
            "pushZeroInsets" => {
                self.push_indexed_command_results(cmd, 4);
                None
            }
            "pushFontMetrics" => {
                let font = self.pop_or_unknown();
                self.push_indexed_call_results(cmd, vec![font], 5);
                None
            }
            "viewport_getzoom" => {
                self.push_named_command_results(cmd, &["min", "max"]);
                None
            }
            "viewport_getfov" => {
                self.push_named_command_results(cmd, &["max", "min"]);
                None
            }
            "fullscreen_getmode" => {
                let index = self.pop_or_unknown();
                self.push_named_call_results(cmd, vec![index], &["width", "height"]);
                None
            }
            "window_getinsets" => {
                self.push_indexed_command_results(cmd, 4);
                None
            }
            "if_hassub" => {
                let component = self.pop_or_unknown();
                self.stack
                    .push(self.ui_call_expr("HasSub", vec![component]));
                None
            }
            "if_getnextsubid" => {
                let component = self.pop_or_unknown();
                self.stack
                    .push(self.ui_call_expr("GetNextSubid", vec![component]));
                None
            }
            "if_hassubmodal" => {
                let interface = self.pop_or_unknown();
                let component = self.pop_or_unknown();
                self.stack
                    .push(self.ui_call_expr("HasSubmodal", vec![component, interface]));
                None
            }
            "if_hassuboverlay" => {
                let overlay = self.pop_or_unknown();
                let component = self.pop_or_unknown();
                self.stack
                    .push(self.ui_call_expr("HasSuboverlay", vec![component, overlay]));
                None
            }
            "if_get_gamescreen" => {
                self.stack.push(self.ui_call_expr("GetGamescreen", vec![]));
                None
            }
            "defaultminimenu"
            | "minimenu_close"
            | "force_interface_drag"
            | "cancel_interface_drag"
            | "targetmode_cancel" => {
                return Some(RecoveredStmt::Expression(self.command_call(cmd, vec![])));
            }

            // ── CC / UI ops ──
            cmd if is_interface_hook_opcode(cmd) => {
                let has_component = cmd.starts_with("if_");
                let component = has_component.then(|| self.pop_or_unknown());
                let descriptor = self.pop_or_unknown();
                let callback = self.recover_callback_literal(cmd, &descriptor);
                let mut arguments = Vec::with_capacity(1 + usize::from(has_component));
                arguments.push(callback.unwrap_or(descriptor));
                if let Some(component) = component {
                    arguments.push(component);
                }
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: format!("UI.{}", sanitize_camel(&cmd[3..])),
                    })),
                    arguments,
                })));
            }
            "cc_create" => {
                let child_id = self.pop_or_unknown();
                let component_type = self.pop_or_unknown();
                let parent = self.pop_or_unknown();
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "UI.create".to_string(),
                    })),
                    arguments: vec![parent, component_type, child_id],
                })));
            }
            "cc_delete" => {
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "UI.delete".to_string(),
                    })),
                    arguments: vec![],
                })));
            }
            "cc_deleteall" => {
                let component = self.pop_or_unknown();
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "UI.deleteAll".to_string(),
                    })),
                    arguments: vec![component],
                })));
            }
            "cc_find" => {
                let child_id = self.pop_or_unknown();
                let component = self.pop_or_unknown();
                self.stack.push(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "UI.find".to_string(),
                    })),
                    arguments: vec![component, child_id],
                }));
                None
            }
            "if_find" => {
                let component = self.pop_or_unknown();
                self.stack.push(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "UI.find".to_string(),
                    })),
                    arguments: vec![component],
                }));
                None
            }
            "cc_sendtofront" => {
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "UI.sendToFront".to_string(),
                    })),
                    arguments: vec![],
                })));
            }
            "cc_sendtoback" => {
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "UI.sendToBack".to_string(),
                    })),
                    arguments: vec![],
                })));
            }
            "if_sendtofront" => {
                let component = self.pop_or_unknown();
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "UI.sendToFront".to_string(),
                    })),
                    arguments: vec![component],
                })));
            }
            "if_sendtoback" => {
                let component = self.pop_or_unknown();
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "UI.sendToBack".to_string(),
                    })),
                    arguments: vec![component],
                })));
            }
            "if_gettext" => {
                let id = self.pop_or_unknown();
                let expr = Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "UI.getText".to_string(),
                    })),
                    arguments: vec![id],
                });
                self.stack.push(expr);
                None
            }
            "cc_settext" => {
                return Some(self.ui_call("UI.setText", 1));
            }
            "cc_setgraphic" => {
                return Some(self.ui_call("UI.setGraphic", 1));
            }
            "cc_sethide" => {
                return Some(self.ui_call("UI.setHide", 1));
            }
            "cc_setcolour" => {
                return Some(self.ui_call("UI.setColour", 1));
            }
            "cc_setfill" => return Some(self.ui_call("UI.setFill", 1)),
            "cc_settrans" => return Some(self.ui_call("UI.setTrans", 1)),
            "cc_setlinewid" => return Some(self.ui_call("UI.setLineWid", 1)),
            "cc_setmodel" => return Some(self.ui_call("UI.setModel", 1)),
            "cc_setscrollpos" => return Some(self.ui_call("UI.setScrollPos", 2)),
            "cc_setscrollsize" => return Some(self.ui_call("UI.setScrollSize", 2)),
            "cc_setaspect" => return Some(self.ui_call("UI.setAspect", 2)),
            "cc_setposition" => return Some(self.ui_call("UI.setPosition", 4)),
            "cc_setsize" => {
                return Some(self.ui_call("UI.setSize", 4));
            }
            "cc_setmodelorigin" => return Some(self.ui_call("UI.setModelOrigin", 2)),
            "cc_setmodelangle" => return Some(self.ui_call("UI.setModelAngle", 6)),
            "cc_setmodelzoom" => return Some(self.ui_call("UI.setModelZoom", 1)),
            "cc_setmodelorthog" => return Some(self.ui_call("UI.setModelOrthog", 1)),
            "cc_setmodeltint" => return Some(self.ui_call("UI.setModelTint", 4)),
            "cc_setmodellighting" => return Some(self.ui_call("UI.setModelLighting", 10)),
            "cc_resetmodellighting" => return Some(self.ui_call("UI.resetModelLighting", 0)),
            "cc_settextfont" => return Some(self.ui_call("UI.setTextFont", 1)),
            "cc_settextalign" => return Some(self.ui_call("UI.setTextAlign", 3)),
            "cc_settextshadow" => return Some(self.ui_call("UI.setTextShadow", 1)),
            "cc_settextantimacro" => return Some(self.ui_call("UI.setTextAntiMacro", 1)),
            "cc_setoutline" => return Some(self.ui_call("UI.setOutline", 1)),
            "cc_setgraphicshadow" => return Some(self.ui_call("UI.setGraphicShadow", 1)),
            "cc_setclickmask" => return Some(self.ui_call("UI.setClickMask", 1)),
            "cc_setheld" => return Some(self.ui_call("UI.setHeld", 1)),
            "cc_setfontmono" => return Some(self.ui_call("UI.setFontMono", 1)),
            "cc_setparam" => return Some(self.ui_call("UI.setParam", 2)),
            "cc_setparam_int" => return Some(self.ui_call("UI.setParamInt", 2)),
            "cc_setparam_string" => return Some(self.ui_call("UI.setParamString", 2)),
            _ => {
                match categorize(cmd) {
                    OpcodeCategory::CC | OpcodeCategory::IF => {
                        let mut args: Vec<Expression> =
                            (0..effect.pops).map(|_| self.pop_or_unknown()).collect();
                        args.reverse();
                        let call = Expression::Call(CallExpr {
                            callee: Box::new(Expression::Identifier(Identifier {
                                name: format!("UI.{}", sanitize_camel(&cmd[3..])),
                            })),
                            arguments: args,
                        });
                        // A getter (cc_getwidth, if_gettext, ...) produces a
                        // value: push it so a downstream consumer or assignment
                        // picks it up, instead of stranding the operands. A void
                        // UI setter (pushes == 0) stays a statement.
                        if effect.pushes > 0 {
                            self.stack.push(call);
                            return None;
                        }
                        return Some(RecoveredStmt::Expression(call));
                    }
                    OpcodeCategory::Push => {
                        self.stack.push(operand_expr(op));
                    }
                    OpcodeCategory::Pop => {
                        let _val = self.stack.pop();
                    }
                    OpcodeCategory::Branch | OpcodeCategory::Other => {
                        // define_array is a known op that falls through to Other
                        if let OperandNode::Array(id) = op
                            && cmd == "define_array"
                        {
                            return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                                callee: Box::new(Expression::Identifier(Identifier {
                                    name: format!("define_array_{id}"),
                                })),
                                arguments: vec![],
                            })));
                        }
                        // Unknown: emit as call if it consumes or produces stack values
                        if effect.pops > 0 {
                            let mut args: Vec<Expression> =
                                (0..effect.pops).map(|_| self.pop_or_unknown()).collect();
                            args.reverse();
                            if effect.pushes > 0 {
                                let expr = Expression::Call(CallExpr {
                                    callee: Box::new(Expression::Identifier(Identifier {
                                        name: sanitize_command(cmd),
                                    })),
                                    arguments: args,
                                });
                                self.stack.push(expr);
                            } else {
                                return Some(RecoveredStmt::Expression(Expression::Call(
                                    CallExpr {
                                        callee: Box::new(Expression::Identifier(Identifier {
                                            name: sanitize_command(cmd),
                                        })),
                                        arguments: args,
                                    },
                                )));
                            }
                        } else if effect.pushes > 0 {
                            self.stack.push(Expression::Call(CallExpr {
                                callee: Box::new(Expression::Identifier(Identifier {
                                    name: sanitize_command(cmd),
                                })),
                                arguments: vec![],
                            }));
                        }
                    }
                }
                None
            }
        }
    }

    fn pop_expr(&mut self) -> Option<Expression> {
        self.stack.pop()
    }

    fn pop_or_unknown(&mut self) -> Expression {
        self.stack.pop().unwrap_or_else(|| {
            Expression::Call(CallExpr {
                callee: Box::new(Expression::Identifier(Identifier {
                    name: "pop".to_string(),
                })),
                arguments: vec![],
            })
        })
    }

    fn pop_args(&mut self, count: usize) -> Vec<Expression> {
        let mut args = Vec::with_capacity(count);
        for _ in 0..count {
            args.push(self.pop_or_unknown());
        }
        args.reverse();
        args
    }

    fn ui_call(&mut self, name: &str, arg_count: usize) -> RecoveredStmt {
        RecoveredStmt::Expression(Expression::Call(CallExpr {
            callee: Box::new(Expression::Identifier(Identifier {
                name: name.to_string(),
            })),
            arguments: self.pop_args(arg_count),
        }))
    }

    fn ui_call_expr(&self, method: &str, arguments: Vec<Expression>) -> Expression {
        Expression::Call(CallExpr {
            callee: Box::new(Expression::Identifier(Identifier {
                name: format!("UI.{method}"),
            })),
            arguments,
        })
    }

    fn command_call(&self, cmd: &str, arguments: Vec<Expression>) -> Expression {
        Expression::Call(CallExpr {
            callee: Box::new(Expression::Identifier(Identifier {
                name: sanitize_command(cmd),
            })),
            arguments,
        })
    }

    fn push_named_command_results(&mut self, cmd: &str, properties: &[&str]) {
        let call = self.command_call(cmd, vec![]);
        for property in properties {
            self.stack
                .push(Expression::PropertyAccess(super::ast::PropertyAccess {
                    object: Box::new(call.clone()),
                    property: (*property).to_string(),
                }));
        }
    }

    fn push_named_call_results(
        &mut self,
        cmd: &str,
        arguments: Vec<Expression>,
        properties: &[&str],
    ) {
        let call = self.command_call(cmd, arguments);
        for property in properties {
            self.stack
                .push(Expression::PropertyAccess(super::ast::PropertyAccess {
                    object: Box::new(call.clone()),
                    property: (*property).to_string(),
                }));
        }
    }

    fn push_indexed_command_results(&mut self, cmd: &str, count: usize) {
        let call = self.command_call(cmd, vec![]);
        for index in 0..count {
            self.stack
                .push(Expression::ArrayAccess(super::ast::ArrayAccess {
                    array: Box::new(call.clone()),
                    index: Box::new(Expression::NumberLiteral(NumberLiteral {
                        value: index as i32,
                    })),
                }));
        }
    }

    fn push_indexed_call_results(&mut self, cmd: &str, arguments: Vec<Expression>, count: usize) {
        let call = self.command_call(cmd, arguments);
        for index in 0..count {
            self.stack
                .push(Expression::ArrayAccess(super::ast::ArrayAccess {
                    array: Box::new(call.clone()),
                    index: Box::new(Expression::NumberLiteral(NumberLiteral {
                        value: index as i32,
                    })),
                }));
        }
    }

    fn recover_callback_literal(
        &mut self,
        cmd: &str,
        descriptor: &Expression,
    ) -> Option<Expression> {
        let Expression::StringLiteral(descriptor) = descriptor else {
            return None;
        };

        let raw_descriptor = descriptor.value.clone();
        let mut signature = raw_descriptor.as_str();
        let watchers = if let Some(stripped) = signature.strip_suffix('Y') {
            signature = stripped;
            let count = literal_usize(&self.pop_or_unknown())
                .filter(|count| *count <= self.stack.len())
                .filter(|count| *count <= MAX_CALLBACK_WATCHERS)?;
            let mut watchers = Vec::with_capacity(count);
            for _ in 0..count {
                watchers.push(self.pop_or_unknown());
            }
            watchers.reverse();
            watchers
                .into_iter()
                .map(|watcher| hook_watcher_name(cmd, &watcher, self.var_names))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let mut arguments = Vec::with_capacity(signature.chars().count());
        for _ in signature.chars() {
            arguments.push(self.pop_or_unknown());
        }
        arguments.reverse();

        let script = self.pop_or_unknown();
        let script_id = literal_i32(&script);

        Some(Expression::CallbackLiteral(CallbackLiteral {
            script: callback_script_name(&script, self.script_catalog),
            script_id,
            raw_descriptor,
            arguments,
            watchers,
        }))
    }
}

fn local_type(cmd: &str) -> (&'static str, &'static str) {
    if cmd.contains("long") {
        ("local_long", "bigint")
    } else if cmd.contains("string") || cmd.contains("obj") {
        ("local_obj", "string")
    } else {
        ("local_int", "number")
    }
}

fn operand_expr(op: &OperandNode) -> Expression {
    match op {
        OperandNode::Int(v) => Expression::NumberLiteral(NumberLiteral { value: *v }),
        OperandNode::Long(v) => Expression::BigIntLiteral(super::ast::BigIntLiteral { value: *v }),
        OperandNode::String(s) => Expression::StringLiteral(StringLiteral { value: s.clone() }),
        OperandNode::Local(idx) => Expression::Identifier(Identifier {
            name: format!("local_{idx}"),
        }),
        OperandNode::VarRef(vr) => {
            let name = vr
                .name
                .clone()
                .unwrap_or_else(|| format!("var_{}:{}", vr.domain.as_label(), vr.id));
            Expression::Identifier(Identifier { name })
        }
        OperandNode::VarBitRef(vbr) => {
            let name = vbr
                .name
                .clone()
                .unwrap_or_else(|| format!("varbit_{}", vbr.id));
            Expression::Identifier(Identifier { name })
        }
        OperandNode::Array(id) => {
            let arr = Expression::Identifier(Identifier {
                name: format!("array_{id}"),
            });
            Expression::ArrayAccess(super::ast::ArrayAccess {
                array: Box::new(arr),
                index: Box::new(Expression::Identifier(Identifier {
                    name: "idx".to_string(),
                })),
            })
        }
        _ => Expression::Call(CallExpr {
            callee: Box::new(Expression::Identifier(Identifier {
                name: "pop".to_string(),
            })),
            arguments: vec![],
        }),
    }
}

fn parse_callback_literal(value: &str) -> Option<CallbackLiteral> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some((script, watchers)) = trimmed.split_once('{') {
        let watchers = watchers.strip_suffix('}')?;
        let watchers = watchers
            .split(',')
            .map(str::trim)
            .filter(|watcher| !watcher.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        return Some(CallbackLiteral {
            script: script.trim().to_string(),
            script_id: None,
            raw_descriptor: String::new(),
            arguments: Vec::new(),
            watchers,
        });
    }

    if (trimmed.starts_with("script") || trimmed.contains('_'))
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Some(CallbackLiteral {
            script: trimmed.to_string(),
            script_id: None,
            raw_descriptor: String::new(),
            arguments: Vec::new(),
            watchers: Vec::new(),
        });
    }

    None
}

fn is_interface_hook_opcode(cmd: &str) -> bool {
    (cmd.starts_with("if_") || cmd.starts_with("cc_")) && cmd.contains("_seton")
}

fn literal_usize(expr: &Expression) -> Option<usize> {
    usize::try_from(literal_i32(expr)?).ok()
}

fn literal_i32(expr: &Expression) -> Option<i32> {
    match expr {
        Expression::NumberLiteral(number) => Some(number.value),
        Expression::PropertyAccess(property) => {
            property.property.strip_prefix("KEY_")?.parse().ok()
        }
        _ => None,
    }
}

fn callback_script_name(expr: &Expression, script_catalog: &super::ScriptCatalog) -> String {
    if let Some(raw_id) = literal_i32(expr) {
        return script_catalog
            .resolve_call_target(raw_id)
            .map(|target| target.export_name.clone())
            .unwrap_or_else(|| format!("script{raw_id}"));
    }

    match expr {
        Expression::Identifier(identifier) => identifier.name.clone(),
        Expression::StringLiteral(string) => string.value.clone(),
        _ => expr_str(expr),
    }
}

fn hook_watcher_name(
    cmd: &str,
    expr: &Expression,
    var_names: &std::collections::HashMap<(VarDomain, u16), String>,
) -> String {
    let Some(raw_id) = literal_i32(expr) else {
        return expr_str(expr);
    };
    let Ok(id) = u16::try_from(raw_id) else {
        return expr_str(expr);
    };

    match cmd {
        "if_setonvartransmit" | "cc_setonvartransmit" => {
            let fallback = format!("varplayerint_{id}");
            match var_names.get(&(VarDomain::Player, id)) {
                Some(name) if name != &format!("varplayer_{id}") => name.clone(),
                _ => fallback,
            }
        }
        "if_setoninvtransmit" | "cc_setoninvtransmit" => format!("inv_{id}"),
        "if_setonstattransmit" | "cc_setonstattransmit" => format!("stat_{id}"),
        "if_setonvarctransmit" | "cc_setonvarctransmit" => format!("varc_{id}"),
        "if_setonvarcstrtransmit" | "cc_setonvarcstrtransmit" => format!("varcstr_{id}"),
        _ => expr_str(expr),
    }
}

fn sanitize_command(cmd: &str) -> String {
    super::sanitize_ts_ident(&cmd.replace('_', ""))
}

fn sanitize_camel(s: &str) -> String {
    let mut out = String::new();
    let mut capitalize = true;
    for c in s.chars() {
        if c == '_' {
            capitalize = true;
        } else if capitalize {
            out.push(c.to_ascii_uppercase());
            capitalize = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn expr_str(expr: &Expression) -> String {
    match expr {
        Expression::NumberLiteral(n) => n.value.to_string(),
        Expression::BigIntLiteral(n) => format!("{}n", n.value),
        Expression::Identifier(id) => id.name.clone(),
        Expression::StringLiteral(s) => format!("\"{}\"", s.value),
        Expression::BooleanLiteral(b) => b.value.to_string(),
        Expression::ArrayAccess(array) => {
            format!("{}[{}]", expr_str(&array.array), expr_str(&array.index))
        }
        Expression::PropertyAccess(property) => {
            format!("{}.{}", expr_str(&property.object), property.property)
        }
        Expression::Call(call) => {
            let arguments = call
                .arguments
                .iter()
                .map(expr_str)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({arguments})", expr_str(&call.callee))
        }
        Expression::CallbackLiteral(callback) => {
            format!(
                "callback(\"{}\", [{}])",
                callback.script,
                callback.watchers.join(", ")
            )
        }
        Expression::BinaryOperation(binary) => {
            format!(
                "({} {} {})",
                expr_str(&binary.left),
                binary.op.as_str(),
                expr_str(&binary.right)
            )
        }
        Expression::UnaryOperation(unary) => {
            format!("({}{})", unary.op.as_str(), expr_str(&unary.operand))
        }
        Expression::PushOperation(push) => format!("push({})", expr_str(&push.value)),
        Expression::PopOperation(_) => "pop()".to_string(),
        Expression::GotoExpr(goto) => format!("goto({})", goto.target),
    }
}

#[cfg(test)]
mod tests {
    use super::{ExprRecovery, RecoveredStmt};
    use crate::transpile::ScriptCatalog;
    use crate::transpile::ast::{Expression, InstructionNode, OperandNode};
    use std::collections::HashMap;

    #[test]
    fn invalid_join_string_count_fails_soft() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::String("hello".to_string()),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "join_string".to_string(),
                operand: OperandNode::Count(1_543_595_008),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "return".to_string(),
                operand: OperandNode::Int(0),
            },
        ];

        let recovered = ExprRecovery::new(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        assert!(matches!(
            &recovered[1],
            Some(RecoveredStmt::Comment(text)) if text.contains("invalid join_string count")
        ));
        assert!(matches!(
            &recovered[2],
            Some(RecoveredStmt::Return(Some(_)))
        ));
    }

    #[test]
    fn cc_delete_uses_active_component_without_stack_argument() {
        let instructions = vec![InstructionNode {
            index: 0,
            opcode: 0,
            command: "cc_delete".to_string(),
            operand: OperandNode::Byte(0),
        }];

        let recovered = ExprRecovery::new(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        assert!(matches!(
            &recovered[0],
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if call.arguments.is_empty()
        ));
    }

    #[test]
    fn cc_find_pushes_booleanish_result_expression() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(100),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(7),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "cc_find".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "return".to_string(),
                operand: OperandNode::Int(0),
            },
        ];

        let recovered = ExprRecovery::new(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        assert!(matches!(
            &recovered[3],
            Some(RecoveredStmt::Return(Some(Expression::Call(call))))
                if call.arguments.len() == 2
        ));
    }

    #[test]
    fn cc_settext_uses_active_component_without_component_argument() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::String("hello".to_string()),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "cc_settext".to_string(),
                operand: OperandNode::Byte(0),
            },
        ];

        let recovered = ExprRecovery::new(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        assert!(matches!(
            &recovered[1],
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if call.arguments.len() == 1
        ));
    }

    #[test]
    fn cc_setposition_keeps_all_four_runtime_arguments() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(10),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(20),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(1),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(2),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "cc_setposition".to_string(),
                operand: OperandNode::Byte(0),
            },
        ];

        let recovered = ExprRecovery::new(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        assert!(matches!(
            &recovered[4],
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if call.arguments.len() == 4
        ));
    }

    #[test]
    fn oc_param_pushes_generic_call_expression() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(100),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(7),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "oc_param".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "return".to_string(),
                operand: OperandNode::Byte(0),
            },
        ];

        let recovered = ExprRecovery::new(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        assert!(matches!(
            &recovered[3],
            Some(RecoveredStmt::Return(Some(Expression::Call(call))))
                if call.arguments.len() == 2
        ));
    }

    #[test]
    fn player_group_same_world_var_pushes_generic_call_expression() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(1),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(1),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(7),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "player_group_member_get_same_world_var".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "return".to_string(),
                operand: OperandNode::Byte(0),
            },
        ];

        let recovered = ExprRecovery::new(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        assert!(matches!(
            &recovered[4],
            Some(RecoveredStmt::Return(Some(Expression::Call(call))))
                if call.arguments.len() == 3
        ));
    }

    #[test]
    fn notifications_sendlocal_pops_four_args_and_pushes_result_expression() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::String("title".to_string()),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::String("body".to_string()),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(1),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(2),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "notifications_sendlocal".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 5,
                opcode: 0,
                command: "return".to_string(),
                operand: OperandNode::Byte(0),
            },
        ];

        let recovered = ExprRecovery::new(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        assert!(matches!(
            &recovered[5],
            Some(RecoveredStmt::Return(Some(Expression::Call(call))))
                if call.arguments.len() == 4
        ));
    }

    #[test]
    fn db_helpers_use_runtime_stack_args_instead_of_operand_lookup() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(42),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "db_listall".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(7),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "db_getrowtable".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "return".to_string(),
                operand: OperandNode::Byte(0),
            },
        ];

        let recovered = ExprRecovery::new(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        assert!(matches!(
            &recovered[4],
            Some(RecoveredStmt::Return(Some(Expression::Call(call))))
                if matches!(&*call.callee, Expression::Identifier(id) if id.name == "dbgetrowtable")
                    && call.arguments.len() == 1
        ));
        assert!(recovered[1].is_none());
    }

    #[test]
    fn minimenu_entry_helper_pushes_named_properties_in_vm_order() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "get_active_minimenu_entry".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: OperandNode::Local(0),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: OperandNode::Local(1),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: OperandNode::Local(2),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(0),
            },
        ];

        let recovered = ExprRecovery::new(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        for (slot, property) in [
            (&recovered[1], "questIconSuffix"),
            (&recovered[2], "opBase"),
            (&recovered[3], "op"),
            (&recovered[4], "entityType"),
        ] {
            assert!(matches!(
                slot,
                Some(RecoveredStmt::Assignment { value: Expression::PropertyAccess(access), .. })
                    if access.property == property
                        && matches!(&*access.object, Expression::Call(call)
                            if matches!(&*call.callee, Expression::Identifier(id) if id.name == "getactiveminimenuentry"))
            ));
        }
    }

    #[test]
    fn minimenu_target_helper_preserves_multivalue_stack_order() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "get_minimenu_target".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: OperandNode::Local(0),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: OperandNode::Local(1),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(0),
            },
        ];

        let recovered = ExprRecovery::new(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        for (slot, index) in [(&recovered[1], 2), (&recovered[2], 1), (&recovered[3], 0)] {
            assert!(matches!(
                slot,
                Some(RecoveredStmt::Assignment { value: Expression::ArrayAccess(access), .. })
                    if matches!(&*access.array, Expression::Call(call)
                        if matches!(&*call.callee, Expression::Identifier(id) if id.name == "getminimenutarget"))
                        && matches!(&*access.index, Expression::NumberLiteral(crate::transpile::ast::NumberLiteral { value }) if *value == index)
            ));
        }
    }

    #[test]
    fn noarg_minimenu_commands_emit_call_statements() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "defaultminimenu".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "minimenu_close".to_string(),
                operand: OperandNode::Byte(0),
            },
        ];

        let recovered = ExprRecovery::new(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        for (slot, callee) in [
            (&recovered[0], "defaultminimenu"),
            (&recovered[1], "minimenuclose"),
        ] {
            assert!(matches!(
                slot,
                Some(RecoveredStmt::Expression(Expression::Call(call)))
                    if matches!(&*call.callee, Expression::Identifier(id) if id.name == callee)
                        && call.arguments.is_empty()
            ));
        }
    }

    #[test]
    fn window_getinsets_preserves_multivalue_stack_order() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "window_getinsets".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(0),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(1),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(2),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(3),
            },
        ];

        let recovered = ExprRecovery::new(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        for (slot, index) in [
            (&recovered[1], 3),
            (&recovered[2], 2),
            (&recovered[3], 1),
            (&recovered[4], 0),
        ] {
            assert!(matches!(
                slot,
                Some(RecoveredStmt::Assignment { value: Expression::ArrayAccess(access), .. })
                    if matches!(&*access.array, Expression::Call(call)
                        if matches!(&*call.callee, Expression::Identifier(id) if id.name == "windowgetinsets"))
                        && matches!(&*access.index, Expression::NumberLiteral(crate::transpile::ast::NumberLiteral { value }) if *value == index)
            ));
        }
    }

    #[test]
    fn interface_misc_getters_push_result_expressions() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(42),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "if_hassub".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(42),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(7),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "if_hassubmodal".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 5,
                opcode: 0,
                command: "if_get_gamescreen".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 6,
                opcode: 0,
                command: "return".to_string(),
                operand: OperandNode::Byte(0),
            },
        ];

        let recovered = ExprRecovery::new(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        assert!(matches!(
            &recovered[6],
            Some(RecoveredStmt::Return(Some(Expression::Call(call))))
                if matches!(&*call.callee, Expression::Identifier(id) if id.name == "UI.GetGamescreen")
        ));
        assert!(recovered[4].is_none());
        assert!(recovered[1].is_none());
    }

    #[test]
    fn interface_misc_noarg_commands_emit_call_statements() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "if_close".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "force_interface_drag".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "cancel_interface_drag".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "targetmode_cancel".to_string(),
                operand: OperandNode::Byte(0),
            },
        ];

        let recovered = ExprRecovery::new(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        for (slot, callee) in [
            (&recovered[0], "UI.Close"),
            (&recovered[1], "forceinterfacedrag"),
            (&recovered[2], "cancelinterfacedrag"),
            (&recovered[3], "targetmodecancel"),
        ] {
            assert!(matches!(
                slot,
                Some(RecoveredStmt::Expression(Expression::Call(call)))
                    if matches!(&*call.callee, Expression::Identifier(id) if id.name == callee)
                        && call.arguments.is_empty()
            ));
        }
    }

    #[test]
    fn display_helpers_preserve_named_and_indexed_stack_results() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "pushCanvasSize".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(0),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(1),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(2),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "fullscreen_getmode".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 5,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(2),
            },
            InstructionNode {
                index: 6,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(3),
            },
            InstructionNode {
                index: 7,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(9),
            },
            InstructionNode {
                index: 8,
                opcode: 0,
                command: "pushFontMetrics".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 9,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(4),
            },
            InstructionNode {
                index: 10,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(5),
            },
            InstructionNode {
                index: 11,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(6),
            },
            InstructionNode {
                index: 12,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(7),
            },
            InstructionNode {
                index: 13,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(8),
            },
        ];

        let recovered = ExprRecovery::new(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        for (slot, callee, property) in [
            (&recovered[1], "pushCanvasSize", "height"),
            (&recovered[2], "pushCanvasSize", "width"),
            (&recovered[5], "fullscreengetmode", "height"),
            (&recovered[6], "fullscreengetmode", "width"),
        ] {
            assert!(matches!(
                slot,
                Some(RecoveredStmt::Assignment { value: Expression::PropertyAccess(access), .. })
                    if access.property == property
                        && matches!(&*access.object, Expression::Call(call)
                            if matches!(&*call.callee, Expression::Identifier(id) if id.name == callee))
            ));
        }

        for (slot, index) in [
            (&recovered[9], 4),
            (&recovered[10], 3),
            (&recovered[11], 2),
            (&recovered[12], 1),
            (&recovered[13], 0),
        ] {
            assert!(matches!(
                slot,
                Some(RecoveredStmt::Assignment { value: Expression::ArrayAccess(access), .. })
                    if matches!(&*access.array, Expression::Call(call)
                        if matches!(&*call.callee, Expression::Identifier(id) if id.name == "pushFontMetrics"))
                        && matches!(&*access.index, Expression::NumberLiteral(crate::transpile::ast::NumberLiteral { value }) if *value == index)
            ));
        }
    }

    #[test]
    fn worldlist_helpers_preserve_named_stack_results() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "worldlist_start".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: OperandNode::Local(0),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(0),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(1),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: OperandNode::Local(1),
            },
            InstructionNode {
                index: 5,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(2),
            },
            InstructionNode {
                index: 6,
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: OperandNode::Local(2),
            },
            InstructionNode {
                index: 7,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(3),
            },
            InstructionNode {
                index: 8,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(4),
            },
            InstructionNode {
                index: 9,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(302),
            },
            InstructionNode {
                index: 10,
                opcode: 0,
                command: "worldlist_specific".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 11,
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: OperandNode::Local(3),
            },
            InstructionNode {
                index: 12,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(5),
            },
            InstructionNode {
                index: 13,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(6),
            },
            InstructionNode {
                index: 14,
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: OperandNode::Local(4),
            },
            InstructionNode {
                index: 15,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(7),
            },
            InstructionNode {
                index: 16,
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: OperandNode::Local(5),
            },
            InstructionNode {
                index: 17,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(8),
            },
        ];

        let recovered = ExprRecovery::new(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        for (slot, callee, property) in [
            (&recovered[1], "worldliststart", "host"),
            (&recovered[2], "worldliststart", "ping"),
            (&recovered[3], "worldliststart", "players"),
            (&recovered[4], "worldliststart", "countryName"),
            (&recovered[5], "worldliststart", "countryId"),
            (&recovered[6], "worldliststart", "activity"),
            (&recovered[7], "worldliststart", "flags"),
            (&recovered[8], "worldliststart", "id"),
        ] {
            assert!(matches!(
                slot,
                Some(RecoveredStmt::Assignment { value: Expression::PropertyAccess(access), .. })
                    if access.property == property
                        && matches!(&*access.object, Expression::Call(call)
                            if matches!(&*call.callee, Expression::Identifier(id) if id.name == callee)
                                && call.arguments.is_empty())
            ));
        }

        for (slot, property) in [
            (&recovered[11], "host"),
            (&recovered[12], "ping"),
            (&recovered[13], "players"),
            (&recovered[14], "countryName"),
            (&recovered[15], "countryId"),
            (&recovered[16], "activity"),
            (&recovered[17], "flags"),
        ] {
            assert!(matches!(
                slot,
                Some(RecoveredStmt::Assignment { value: Expression::PropertyAccess(access), .. })
                    if access.property == property
                        && matches!(&*access.object, Expression::Call(call)
                            if matches!(&*call.callee, Expression::Identifier(id) if id.name == "worldlistspecific")
                                && matches!(call.arguments.as_slice(), [Expression::NumberLiteral(crate::transpile::ast::NumberLiteral { value: 302 })]))
            ));
        }
    }

    #[test]
    fn if_getnextsubid_pushes_ui_getter_expression() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(42),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "if_getnextsubid".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "return".to_string(),
                operand: OperandNode::Byte(0),
            },
        ];

        let recovered = ExprRecovery::new(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        assert!(matches!(
            &recovered[2],
            Some(RecoveredStmt::Return(Some(Expression::Call(call))))
                if matches!(&*call.callee, Expression::Identifier(id) if id.name == "UI.GetNextSubid")
        ));
    }
}
