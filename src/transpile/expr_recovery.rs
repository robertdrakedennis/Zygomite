use super::ast::{
    BinaryOp, CallExpr, CallbackLiteral, Expression, Identifier, InstructionNode, NumberLiteral,
    OperandNode, StringLiteral,
};
use crate::vars::VarDomain;
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;

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

fn stack_effect<SignatureHasher>(
    cmd: &str,
    operand: &OperandNode,
    build: u32,
    script_catalog: &super::ScriptCatalog,
    script_signatures: &std::collections::HashMap<
        super::ScriptId,
        super::ScriptSignature,
        SignatureHasher,
    >,
) -> StackEffect
where
    SignatureHasher: std::hash::BuildHasher,
{
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

        // Array helpers.
        "define_array" => StackEffect { pops: 1, pushes: 0 },
        "array_sort" => StackEffect { pops: 3, pushes: 0 },

        // Control flow
        "branch" => StackEffect { pops: 0, pushes: 0 },
        "branch_if_true" | "branch_if_false" => StackEffect { pops: 1, pushes: 0 },
        "branch_not"
        | "branch_equals"
        | "branch_less_than"
        | "branch_greater_than"
        | "branch_less_than_or_equals"
        | "branch_greater_than_or_equals"
        | "long_branch_not"
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
                    pushes: signature
                        .total_returns()
                        .max(usize::from(signature.return_type != "void")),
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
        | "activeclanchannel_find_listened"
        | "activeclanchannel_getclanname"
        | "activeclanchannel_getrankkick"
        | "activeclanchannel_getranktalk"
        | "activeclanchannel_getusercount"
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
        "activeclanchannel_find_affined" | "activeclansettings_find_affined" => StackEffect {
            pops: usize::from(build >= 938),
            pushes: 1,
        },
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
        "db_find" if build >= 919 => StackEffect { pops: 3, pushes: 0 },
        "db_find" => StackEffect { pops: 2, pushes: 0 },
        "db_find_with_count" if build >= 919 => StackEffect { pops: 3, pushes: 1 },
        "db_find_with_count" => StackEffect { pops: 2, pushes: 1 },
        "db_findnext" => StackEffect { pops: 0, pushes: 1 },
        // DB field arity depends on column schema. Recover one best-effort
        // value here rather than dropping stack behavior entirely.
        "db_getfield" => StackEffect { pops: 3, pushes: 1 },
        "db_getfieldcount" => StackEffect { pops: 2, pushes: 1 },
        "db_find_refine" if build >= 919 => StackEffect { pops: 3, pushes: 1 },
        "db_find_refine" => StackEffect { pops: 2, pushes: 1 },
        "db_find_get" | "db_getrowtable" => StackEffect { pops: 1, pushes: 1 },
        "db_filter_find" => StackEffect { pops: 5, pushes: 1 },
        "db_filter_value" => StackEffect { pops: 4, pushes: 1 },
        "db_filter_unknown" => StackEffect { pops: 1, pushes: 1 },
        "db_filter_combine" => StackEffect { pops: 2, pushes: 1 },
        "db_filter_substring" | "db_filter_column" => StackEffect { pops: 3, pushes: 1 },
        "cam2_setlookatmode" | "cam2_setpositionmode" => StackEffect { pops: 1, pushes: 0 },
        "cam2_setpositionentity_npc" | "cam2_setpositionentity_player" if build >= 919 => {
            StackEffect { pops: 8, pushes: 0 }
        }
        "cam2_setpositionentity_npc" | "cam2_setpositionentity_player" => {
            StackEffect { pops: 7, pushes: 0 }
        }
        "error" => StackEffect { pops: 1, pushes: 0 },
        "create_availablerequest"
        | "create_name_availablerequest"
        | "create_step_reached"
        | "shop_open"
        | "notifications_cancellocal"
        | "shop_purchaseitem"
        | "marketing_sendevent"
        | "marketing_sendanalyticsevent"
        | "marketing_sendattributionevent" => StackEffect { pops: 1, pushes: 0 },
        "cam2_setdepthplanes" | "worldmap_3dview_setloddistance" => {
            StackEffect { pops: 2, pushes: 0 }
        }
        "highlight_reset_category_to_default" | "setminimapmask" => {
            StackEffect { pops: 1, pushes: 0 }
        }
        "highlight_set_localplayer_silhouette_mode" | "unknown_command_65" => {
            StackEffect { pops: 1, pushes: 0 }
        }
        "highlight_set_category_scale" => StackEffect { pops: 2, pushes: 0 },
        "unknown_command_7" => StackEffect { pops: 3, pushes: 0 },
        "unknown_command_62" => StackEffect { pops: 4, pushes: 1 },
        "field5283" => StackEffect { pops: 1, pushes: 1 },
        "field6317" => StackEffect { pops: 1, pushes: 0 },
        "get_entity_bounding_box" | "get_loc_bounding_box" | "get_obj_bounding_box" => {
            StackEffect { pops: 0, pushes: 5 }
        }
        "worldmap_3dview_getcoordfine" => StackEffect { pops: 3, pushes: 1 },
        "worldmap_coordinmap" => StackEffect { pops: 2, pushes: 1 },
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
        | "cc_setlinedirection"
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
        "cc_setobject"
        | "cc_setobject_nonum"
        | "cc_setobject_alwaysnum"
        | "cc_setobject_wearcol"
        | "cc_setobject_wearcol_nonum"
        | "cc_setobject_wearcol_alwaysnum"
        | "cc_setobject_long"
        | "cc_setobject_alwaysnum_long"
        | "cc_setobject_wearcol_long"
        | "cc_setobject_wearcol_alwaysnum_long" => StackEffect { pops: 2, pushes: 0 },
        "cc_setobject_highres" => StackEffect { pops: 1, pushes: 0 },
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
        | "cc_callonresize"
        | "cc_resume_pausebutton"
        | "cc_scriptqueue_clear" => StackEffect { pops: 0, pushes: 0 },
        "cc_deleteall" | "if_sendtofront" | "if_sendtoback" => StackEffect { pops: 1, pushes: 0 },
        "cc_find" => StackEffect { pops: 2, pushes: 1 },
        "cc_find_parent" => StackEffect { pops: 0, pushes: 1 },
        "cc_button_setcantoggle" | "cc_button_settoggled" => StackEffect { pops: 1, pushes: 0 },
        "cc_button_setlinkobjoptions" | "cc_scriptqueue_clear_script" => {
            StackEffect { pops: 2, pushes: 0 }
        }
        "cc_grid_setlayoutparams" => StackEffect { pops: 3, pushes: 0 },
        "cc_button_settextareasizeoffsets" => StackEffect { pops: 4, pushes: 0 },
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
        | "if_setlinedirection"
        | "if_setnoclickthrough"
        | "if_setstylesheet"
        | "if_settiling"
        | "if_resetmodellighting"
        | "if_setonclick"
        | "if_setonvartransmit"
        | "if_setonstocktransmit"
        | "if_setoninvtransmit" => StackEffect { pops: 2, pushes: 0 },
        "if_setscrollpos" | "if_setscrollsize" | "if_setaspect" | "if_setmodelorigin"
        | "if_setparam_int" | "if_setparam_string" => StackEffect { pops: 3, pushes: 0 },
        "if_setobject"
        | "if_setobject_nonum"
        | "if_setobject_alwaysnum"
        | "if_setobject_wearcol"
        | "if_setobject_wearcol_nonum"
        | "if_setobject_wearcol_alwaysnum"
        | "if_setobject_long"
        | "if_setobject_alwaysnum_long"
        | "if_setobject_wearcol_long"
        | "if_setobject_wearcol_alwaysnum_long" => StackEffect { pops: 3, pushes: 0 },
        "if_setobject_highres"
        | "if_button_setcantoggle"
        | "if_button_settoggled"
        | "if_scriptqueue_clear_script" => StackEffect { pops: 2, pushes: 0 },
        "if_settextalign" | "if_setrecol" | "if_setretex" => StackEffect { pops: 4, pushes: 0 },
        "if_button_setlinkobjoptions" => StackEffect { pops: 3, pushes: 0 },
        "if_grid_setlayoutparams" => StackEffect { pops: 4, pushes: 0 },
        "if_setposition" | "if_setsize" | "if_setmodeltint" => StackEffect { pops: 5, pushes: 0 },
        "if_button_settextareasizeoffsets" => StackEffect { pops: 5, pushes: 0 },
        "if_setmodelangle" => StackEffect { pops: 7, pushes: 0 },
        "if_setmodellighting" => StackEffect {
            pops: 11,
            pushes: 0,
        },
        "if_resume_pausebutton" | "if_scriptqueue_clear" => StackEffect { pops: 1, pushes: 0 },

        // Misc known ops
        "cc_create" => StackEffect { pops: 3, pushes: 0 },
        "baseidkit" | "basecolour" | "setobj" => StackEffect { pops: 2, pushes: 0 },
        "setgender" => StackEffect { pops: 1, pushes: 0 },
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

        // `tostring` gained a radix/base argument in build 919. Recovery must
        // model the build-specific arity or it drops the value being converted
        // and shortens every downstream branch target in affected scripts.
        "tostring" if build >= 919 => StackEffect { pops: 2, pushes: 1 },

        // Component getters and value ops (cc_get*/if_get*, tostring, max, min,
        // oc_name, scale, testbit, append, movecoord, ...) are now supplied by
        // the client-extracted opcode table consulted in the default arm. Only
        // `string_length` keeps an explicit entry: its handler's null-check
        // branch makes the static extractor double-count the push (1->2), so the
        // hand value overrides the table.
        "string_length" => StackEffect { pops: 1, pushes: 1 },

        // The explicit arms above are hand-verified and win; for everything else
        // consult the client-extracted opcode table (the long tail of getters,
        // config lookups and value ops) before the coarse categorisation
        // default. A wrong table effect only leaves its scripts gate-blocked,
        // never miscompiles.
        _ => {
            if let Some(effect) = opcode_stack_effect_for_build(cmd, build) {
                StackEffect {
                    pops: effect.total_pops(),
                    pushes: effect.total_pushes(),
                }
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

/// Per-stack pop/push counts of an opcode, extracted from the client.
///
/// Used by the recovery (total counts), the lowerer (result kind via
/// [`OpcodeStackEffect::pushed_type`]) and the validator (typed stack model).
#[derive(Clone, Copy, Default)]
pub struct OpcodeStackEffect {
    pub int_pops: usize,
    pub obj_pops: usize,
    pub long_pops: usize,
    pub int_pushes: usize,
    pub obj_pushes: usize,
    pub long_pushes: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PushedType {
    Int,
    Obj,
    Long,
    None,
    Multi,
}

impl OpcodeStackEffect {
    pub fn total_pops(&self) -> usize {
        self.int_pops + self.obj_pops + self.long_pops
    }

    pub fn total_pushes(&self) -> usize {
        self.int_pushes + self.obj_pushes + self.long_pushes
    }

    /// The stack a single pushed value lands on, for result-kind recovery;
    /// `Multi` when more than one stack is pushed.
    pub fn pushed_type(&self) -> PushedType {
        match (
            self.int_pushes > 0,
            self.obj_pushes > 0,
            self.long_pushes > 0,
        ) {
            (false, false, false) => PushedType::None,
            (true, false, false) => PushedType::Int,
            (false, true, false) => PushedType::Obj,
            (false, false, true) => PushedType::Long,
            _ => PushedType::Multi,
        }
    }
}

/// The client-extracted opcode stack-effect table (see
/// `scripts/extract-stack-effects.py` and `data/stack-effects.txt`). Keyed by
/// command name; build independent.
pub fn opcode_stack_effect(command: &str) -> Option<OpcodeStackEffect> {
    manual_opcode_stack_effect(command).or_else(|| opcode_stack_effect_full().get(command).copied())
}

pub fn opcode_stack_effect_for_build(command: &str, build: u32) -> Option<OpcodeStackEffect> {
    match command {
        "activeclanchannel_find_affined" | "activeclansettings_find_affined" if build >= 938 => {
            Some(int_effect(1, 1))
        }
        "tostring" if build >= 919 => Some(OpcodeStackEffect {
            int_pops: 2,
            obj_pushes: 1,
            ..OpcodeStackEffect::default()
        }),
        "tostring_long" if build >= 936 => Some(OpcodeStackEffect {
            int_pops: 1,
            long_pops: 1,
            obj_pushes: 1,
            ..OpcodeStackEffect::default()
        }),
        "lobby_enterlobby_social_network" if build >= 919 => Some(int_effect(4, 0)),
        "cam2_setpositionentity_npc" | "cam2_setpositionentity_player" if build >= 919 => {
            Some(int_effect(8, 0))
        }
        "cam2_setpositionentity_npc" | "cam2_setpositionentity_player" => Some(int_effect(7, 0)),
        _ => opcode_stack_effect(command),
    }
}

fn int_effect(pops: usize, pushes: usize) -> OpcodeStackEffect {
    OpcodeStackEffect {
        int_pops: pops,
        int_pushes: pushes,
        ..OpcodeStackEffect::default()
    }
}

fn manual_opcode_stack_effect(command: &str) -> Option<OpcodeStackEffect> {
    match command {
        "activeclanchannel_find_affined"
        | "activeclanchannel_find_listened"
        | "activeclansettings_find_affined"
        | "activeclansettings_find_listened" => Some(int_effect(0, 1)),
        "baseidkit" | "basecolour" | "setobj" => Some(int_effect(2, 0)),
        "setgender" => Some(int_effect(1, 0)),
        "cc_clearscripthooks" => Some(int_effect(0, 0)),
        "cam2_setfieldofview" | "cam2_setcollisionmode" => Some(int_effect(2, 0)),
        "cam2_setlookatentity_npc" | "cam2_setlookatentity_player" => Some(int_effect(4, 0)),
        "cam2_setlookatacceleration_axis"
        | "cam2_setlookatmaxspeed_axis"
        | "cam2_setpositionacceleration_axis"
        | "cam2_setpositionmaxspeed_axis" => Some(int_effect(4, 0)),
        "cam2_setlinearmovementmode"
        | "cam2_setlookatangularinterpolation"
        | "cam2_setpositionangularinterpolation" => Some(int_effect(1, 0)),
        "cam2_setsnapdistances" => Some(int_effect(3, 0)),
        "cam2_setspringproperties" | "highlight_set_category_colour" => Some(int_effect(4, 0)),
        "chat_setmode_affined_clan"
        | "cutscene2d_stop"
        | "deeplink_clear_index"
        | "picking_setmaxdistance"
        | "unknown_command_27"
        | "unknown_command_44"
        | "unknown_command_48"
        | "unknown_command_50"
        | "unknown_command_53"
        | "unknown_command_55"
        | "worldmap_3dview_enable" => Some(int_effect(1, 0)),
        "federated_login" | "highlight_set_category_mode" | "unknown_command_58" => {
            Some(int_effect(2, 0))
        }
        "unknown_command_16" | "unknown_command_35" => Some(int_effect(4, 0)),
        "unknown_command_77" => Some(int_effect(3, 0)),
        "deeplink_get"
        | "notifications_islocalscheduled"
        | "preload_progress"
        | "stat"
        | "stat_base"
        | "stat_base_actual"
        | "stylesheet_has_parent"
        | "stylesheet_get_parent_id"
        | "unknown_command_81" => Some(int_effect(1, 1)),
        "stylesheet_has_value" => Some(OpcodeStackEffect {
            int_pops: 1,
            obj_pops: 1,
            int_pushes: 1,
            ..OpcodeStackEffect::default()
        }),
        "unknown_command_20" => Some(int_effect(0, 1)),
        "field6563" => Some(int_effect(0, 1)),
        "struct_param" => Some(int_effect(2, 1)),
        "unknown_command_34" | "unknown_command_79" => Some(int_effect(2, 1)),
        "inv_stockbase" => Some(int_effect(2, 1)),
        "unknown_command_103" => Some(OpcodeStackEffect {
            int_pops: 2,
            obj_pops: 1,
            ..OpcodeStackEffect::default()
        }),
        "unknown_command_104" | "unknown_command_105" => Some(int_effect(1, 0)),
        "cam2_setdepthplanes" | "worldmap_3dview_setloddistance" => Some(int_effect(2, 0)),
        "highlight_reset_category_to_default" | "setminimapmask" => Some(int_effect(1, 0)),
        "highlight_set_localplayer_silhouette_mode" | "unknown_command_65" => {
            Some(int_effect(1, 0))
        }
        "highlight_set_category_scale" => Some(int_effect(2, 0)),
        "unknown_command_7" => Some(int_effect(3, 0)),
        "unknown_command_62" => Some(int_effect(4, 1)),
        "field5283" => Some(int_effect(1, 1)),
        "field6317" => Some(int_effect(1, 0)),
        "get_entity_bounding_box" | "get_loc_bounding_box" | "get_obj_bounding_box" => {
            Some(int_effect(0, 5))
        }
        "cc_getcharindexatpos" => Some(int_effect(2, 1)),
        "if_getcharindexatpos" => Some(int_effect(3, 1)),
        "cc_getcharposatindex" => Some(int_effect(1, 2)),
        "if_getcharposatindex" => Some(int_effect(2, 2)),
        "marketing_sendanalyticsevent" | "marketing_sendattributionevent" => {
            Some(OpcodeStackEffect {
                obj_pops: 1,
                ..OpcodeStackEffect::default()
            })
        }
        "resume_countdialog_long" => Some(OpcodeStackEffect {
            obj_pops: 1,
            ..OpcodeStackEffect::default()
        }),
        "worldmap_3dview_getcoordfine" => Some(int_effect(3, 1)),
        "worldmap_coordinmap" => Some(int_effect(2, 1)),
        "stylesheet_get_value" => Some(OpcodeStackEffect {
            int_pops: 2,
            obj_pops: 1,
            int_pushes: 1,
            ..OpcodeStackEffect::default()
        }),
        "if_set2dangle" | "if_sethflip" | "if_setnpcmodel" | "if_setvflip" => {
            Some(int_effect(2, 0))
        }
        "oc_desc" => Some(OpcodeStackEffect {
            int_pops: 1,
            obj_pushes: 1,
            ..OpcodeStackEffect::default()
        }),
        "store_lookup" => Some(OpcodeStackEffect {
            int_pops: 1,
            obj_pops: 1,
            int_pushes: 10,
            long_pushes: 3,
            ..OpcodeStackEffect::default()
        }),
        "store_init" => Some(OpcodeStackEffect {
            obj_pops: 1,
            ..OpcodeStackEffect::default()
        }),
        "steam_setachievement" => Some(OpcodeStackEffect {
            int_pops: 2,
            obj_pops: 1,
            int_pushes: 1,
            ..OpcodeStackEffect::default()
        }),
        "cc_npc_setcustombodymodel"
        | "cc_npc_setcustomheadmodel"
        | "cc_npc_setcustomrecol"
        | "cc_npc_setcustomretex" => Some(int_effect(2, 0)),
        "if_npc_setcustombodymodel"
        | "if_npc_setcustomheadmodel"
        | "if_npc_setcustomrecol"
        | "if_npc_setcustomretex" => Some(int_effect(3, 0)),
        "cc_setcustombodyretex"
        | "cc_setcustombodyrecol"
        | "cc_setcustomheadretex"
        | "cc_setcustomheadrecol" => Some(int_effect(4, 0)),
        "if_setcustombodyretex"
        | "if_setcustombodyrecol"
        | "if_setcustomheadretex"
        | "if_setcustomheadrecol" => Some(int_effect(5, 0)),
        "cc_npc_setcustombodymodel_transformed" => Some(int_effect(10, 0)),
        "if_npc_setcustombodymodel_transformed" => Some(int_effect(11, 0)),
        "cc_createchild" => Some(int_effect(3, 0)),
        "if_createchild" => Some(int_effect(4, 0)),
        "db_listall" => Some(int_effect(1, 1)),
        "db_find_with_count" | "db_find_refine" => Some(int_effect(3, 1)),
        "db_findnext" => Some(int_effect(0, 1)),
        "db_getfield" => Some(int_effect(3, 1)),
        "db_getfieldcount" => Some(int_effect(2, 1)),
        "db_find_get" | "db_getrowtable" => Some(int_effect(1, 1)),
        "db_filter_find" => Some(int_effect(5, 1)),
        "db_filter_value" => Some(int_effect(4, 1)),
        "db_filter_unknown" => Some(int_effect(1, 1)),
        "db_filter_combine" => Some(int_effect(2, 1)),
        "db_filter_column" => Some(int_effect(3, 1)),
        "db_filter_substring" => Some(OpcodeStackEffect {
            int_pops: 2,
            obj_pops: 1,
            int_pushes: 1,
            ..OpcodeStackEffect::default()
        }),
        // The VM handlers route these through interface-component helpers, so
        // stack extraction sees only the final component pop. The command
        // signatures and bytecode use the full option payload.
        "if_setop" => Some(OpcodeStackEffect {
            int_pops: 2,
            obj_pops: 1,
            ..OpcodeStackEffect::default()
        }),
        "if_setopbase" | "if_setpausetext" | "if_settargetverb" => Some(OpcodeStackEffect {
            int_pops: 1,
            obj_pops: 1,
            ..OpcodeStackEffect::default()
        }),
        "if_setopcursor" | "if_setopchar" | "if_setoptkey" | "if_settargetcursors" => {
            Some(int_effect(3, 0))
        }
        "if_setoptchar"
        | "if_setoptkeyignoreheld"
        | "if_setopkeyignoreheld"
        | "if_setdragrenderbehaviour"
        | "if_setdragdeadzone"
        | "if_setdragdeadtime"
        | "if_setmouseovercursor"
        | "if_settargetopcursor"
        | "if_setmaxlines"
        | "if_setmodelanim"
        | "if_setnpchead"
        | "if_setplayerhead_self" => Some(int_effect(2, 0)),
        "if_setopkey" | "if_setopkeyrate" => Some(int_effect(4, 0)),
        "if_setoptkeyrate" => Some(int_effect(3, 0)),
        "if_setdraggable" => Some(int_effect(3, 0)),
        _ => None,
    }
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
            let Some(name) = fields.next() else {
                continue;
            };
            let counts: Vec<usize> = fields.filter_map(|f| f.parse().ok()).collect();
            let [
                int_pops,
                obj_pops,
                long_pops,
                int_pushes,
                obj_pushes,
                long_pushes,
            ] = counts[..]
            else {
                continue;
            };
            map.insert(
                name,
                OpcodeStackEffect {
                    int_pops,
                    obj_pops,
                    long_pops,
                    int_pushes,
                    obj_pushes,
                    long_pushes,
                },
            );
        }
        add_command_signature_effects(&mut map);
        map
    })
}

fn add_command_signature_effects(
    map: &mut std::collections::HashMap<&'static str, OpcodeStackEffect>,
) {
    for line in include_str!("../../data/commands/interface_components.txt")
        .lines()
        .chain(include_str!("../../data/commands/achievement.txt").lines())
        .chain(include_str!("../../data/commands/camera.txt").lines())
        .chain(include_str!("../../data/commands/config_misc.txt").lines())
        .chain(include_str!("../../data/commands/config_quests.txt").lines())
        .chain(include_str!("../../data/commands/core.txt").lines())
        .chain(include_str!("../../data/commands/detail_options.txt").lines())
        .chain(include_str!("../../data/commands/entities.txt").lines())
        .chain(include_str!("../../data/commands/file_system.txt").lines())
        .chain(include_str!("../../data/commands/interface_core.txt").lines())
        .chain(include_str!("../../data/commands/interface_misc.txt").lines())
        .chain(include_str!("../../data/commands/if_anim.txt").lines())
        .chain(include_str!("../../data/commands/input.txt").lines())
        .chain(include_str!("../../data/commands/inventories.txt").lines())
        .chain(include_str!("../../data/commands/login.txt").lines())
        .chain(include_str!("../../data/commands/maths.txt").lines())
        .chain(include_str!("../../data/commands/mini_menu_ops.txt").lines())
        .chain(include_str!("../../data/commands/misc_ops.txt").lines())
        .chain(include_str!("../../data/commands/strings.txt").lines())
        .chain(include_str!("../../data/commands/store.txt").lines())
        .chain(include_str!("../../data/commands/streaming.txt").lines())
        .chain(include_str!("../../data/commands/wikisync.txt").lines())
    {
        let Some((name, effect)) = parse_command_signature_effect(line) else {
            continue;
        };
        map.entry(name).or_insert(effect);
    }
}

fn parse_command_signature_effect(line: &'static str) -> Option<(&'static str, OpcodeStackEffect)> {
    let line = line.trim();
    if line.contains("todo signature") || line.contains("hook ") || line.contains("(hook") {
        return None;
    }
    let line = line.strip_prefix("[command,")?;
    let (name, rest) = line.split_once(']')?;
    if rest.is_empty() {
        return Some((name, OpcodeStackEffect::default()));
    }
    let args = first_parenthesized(rest)?;
    let returns = rest
        .get(args.end_index..)
        .and_then(first_parenthesized)
        .map_or("", |parsed| parsed.contents);
    let mut effect = OpcodeStackEffect::default();
    add_signature_types(args.contents, true, &mut effect);
    add_signature_types(returns, false, &mut effect);
    Some((name, effect))
}

struct ParsedParenthesized<'a> {
    contents: &'a str,
    end_index: usize,
}

fn first_parenthesized(value: &str) -> Option<ParsedParenthesized<'_>> {
    let start = value.find('(')?;
    let relative_end = value[start + 1..].find(')')?;
    let end = start + 1 + relative_end;
    Some(ParsedParenthesized {
        contents: &value[start + 1..end],
        end_index: end + 1,
    })
}

fn add_signature_types(types: &str, pops: bool, effect: &mut OpcodeStackEffect) {
    for field in types
        .split(',')
        .map(str::trim)
        .filter(|field| !field.is_empty())
    {
        let ty = field.split_whitespace().next().unwrap_or(field);
        match ty {
            "string" => {
                if pops {
                    effect.obj_pops += 1;
                } else {
                    effect.obj_pushes += 1;
                }
            }
            "long" => {
                if pops {
                    effect.long_pops += 1;
                } else {
                    effect.long_pushes += 1;
                }
            }
            _ => {
                if pops {
                    effect.int_pops += 1;
                } else {
                    effect.int_pushes += 1;
                }
            }
        }
    }
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
    GotoStack {
        target: usize,
        values: Vec<Expression>,
    },
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
    detect_return_type_from_recovered_inner::<std::collections::hash_map::RandomState>(
        stmts, None, None,
    )
}

pub fn detect_return_type_from_recovered_with_signatures<SignatureHasher>(
    stmts: &[Option<RecoveredStmt>],
    script_catalog: &super::ScriptCatalog,
    script_signatures: &HashMap<super::ScriptId, super::ScriptSignature, SignatureHasher>,
) -> &'static str
where
    SignatureHasher: BuildHasher,
{
    detect_return_type_from_recovered_inner(stmts, Some(script_catalog), Some(script_signatures))
}

pub fn detect_return_counts_from_recovered_with_signatures<SignatureHasher>(
    stmts: &[Option<RecoveredStmt>],
    script_catalog: &super::ScriptCatalog,
    script_signatures: &HashMap<super::ScriptId, super::ScriptSignature, SignatureHasher>,
) -> Option<super::ScriptReturnCounts>
where
    SignatureHasher: BuildHasher,
{
    let context = RecoveredReturnTypeContext {
        script_catalog: Some(script_catalog),
        script_signatures: Some(script_signatures),
    };
    let mut counts = None;
    for stmt in stmts.iter().flatten() {
        let RecoveredStmt::Return(Some(value)) = stmt else {
            continue;
        };
        let next = recovered_return_value_counts(value, &context);
        match counts {
            Some(existing) if existing != next => return None,
            Some(_) => {}
            None => counts = Some(next),
        }
    }
    counts
}

fn detect_return_type_from_recovered_inner<SignatureHasher>(
    stmts: &[Option<RecoveredStmt>],
    script_catalog: Option<&super::ScriptCatalog>,
    script_signatures: Option<&HashMap<super::ScriptId, super::ScriptSignature, SignatureHasher>>,
) -> &'static str
where
    SignatureHasher: BuildHasher,
{
    let context = RecoveredReturnTypeContext {
        script_catalog,
        script_signatures,
    };
    let mut value_kind = None;
    let mut has_mixed_value_returns = false;
    let mut has_void_return = false;

    for stmt in stmts.iter().flatten() {
        if let RecoveredStmt::Return(value) = stmt {
            if let Some(value) = value {
                record_recovered_return_value_kind(
                    &mut value_kind,
                    &mut has_mixed_value_returns,
                    infer_recovered_return_value_kind(value, &context),
                );
            } else {
                has_void_return = true;
            }
        }
    }

    let value_kind = if has_mixed_value_returns {
        Some(RecoveredReturnValueKind::Int)
    } else {
        value_kind
    };
    match (value_kind, has_void_return) {
        (Some(RecoveredReturnValueKind::Int), false) => "number",
        (Some(RecoveredReturnValueKind::Int), true) => "number | void",
        (Some(RecoveredReturnValueKind::Object), false) => "string",
        (Some(RecoveredReturnValueKind::Object), true) => "string | void",
        (Some(RecoveredReturnValueKind::Long), false) => "bigint",
        (Some(RecoveredReturnValueKind::Long), true) => "bigint | void",
        (None, true) => "void",
        (None, false) => "void",
    }
}

#[derive(Debug, Clone, Copy)]
struct RecoveredReturnTypeContext<'a, SignatureHasher> {
    script_catalog: Option<&'a super::ScriptCatalog>,
    script_signatures:
        Option<&'a HashMap<super::ScriptId, super::ScriptSignature, SignatureHasher>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveredReturnValueKind {
    Int,
    Object,
    Long,
}

fn record_recovered_return_value_kind(
    value_kind: &mut Option<RecoveredReturnValueKind>,
    has_mixed_value_returns: &mut bool,
    next: RecoveredReturnValueKind,
) {
    match value_kind {
        Some(existing) if *existing != next => *has_mixed_value_returns = true,
        Some(_) => {}
        None => *value_kind = Some(next),
    }
}

fn infer_recovered_return_value_kind<SignatureHasher>(
    expr: &Expression,
    context: &RecoveredReturnTypeContext<'_, SignatureHasher>,
) -> RecoveredReturnValueKind
where
    SignatureHasher: BuildHasher,
{
    match expr {
        Expression::BigIntLiteral(_) => RecoveredReturnValueKind::Long,
        Expression::StringLiteral(_) => RecoveredReturnValueKind::Object,
        Expression::Identifier(identifier) => recovered_kind_from_identifier(&identifier.name),
        Expression::Call(call) => infer_recovered_call_return_value_kind(call, context),
        _ => RecoveredReturnValueKind::Int,
    }
}

fn recovered_kind_from_identifier(name: &str) -> RecoveredReturnValueKind {
    if name.starts_with("local_obj_") || name.starts_with("arg_obj_") {
        RecoveredReturnValueKind::Object
    } else if name.starts_with("local_long_") || name.starts_with("arg_long_") {
        RecoveredReturnValueKind::Long
    } else {
        RecoveredReturnValueKind::Int
    }
}

fn recovered_return_value_counts<SignatureHasher>(
    expr: &Expression,
    context: &RecoveredReturnTypeContext<'_, SignatureHasher>,
) -> super::ScriptReturnCounts
where
    SignatureHasher: BuildHasher,
{
    if let Expression::Call(call) = expr
        && let Expression::Identifier(identifier) = &*call.callee
    {
        if identifier.name == "stack" {
            let mut counts = super::ScriptReturnCounts::default();
            for value in &call.arguments {
                if is_recovered_void_script_call(value, context) {
                    continue;
                }
                add_recovered_return_kind(
                    &mut counts,
                    infer_recovered_return_value_kind(value, context),
                );
            }
            return counts;
        }
        if let Some(counts) = recovered_script_call_return_counts(&identifier.name, context)
            && counts.total() > 0
        {
            return counts;
        }
    }

    let mut counts = super::ScriptReturnCounts::default();
    add_recovered_return_kind(
        &mut counts,
        infer_recovered_return_value_kind(expr, context),
    );
    counts
}

fn add_recovered_return_kind(
    counts: &mut super::ScriptReturnCounts,
    kind: RecoveredReturnValueKind,
) {
    match kind {
        RecoveredReturnValueKind::Int => counts.int += 1,
        RecoveredReturnValueKind::Object => counts.obj += 1,
        RecoveredReturnValueKind::Long => counts.long += 1,
    }
}

fn infer_recovered_call_return_value_kind(
    call: &CallExpr,
    context: &RecoveredReturnTypeContext<'_, impl BuildHasher>,
) -> RecoveredReturnValueKind {
    let Expression::Identifier(identifier) = &*call.callee else {
        return RecoveredReturnValueKind::Int;
    };
    if identifier.name == "stack" {
        return homogeneous_recovered_stack_return_kind(&call.arguments, context)
            .unwrap_or(RecoveredReturnValueKind::Int);
    }
    if let Some(kind) = recovered_script_call_return_value_kind(&identifier.name, context) {
        return kind;
    }
    match identifier.name.as_str() {
        "append" | "concat" | "lowercase" | "ocname" | "removetags" | "tostring"
        | "tostringlocalised" | "tostringlong" | "uppercase" => RecoveredReturnValueKind::Object,
        _ => RecoveredReturnValueKind::Int,
    }
}

fn recovered_script_call_return_value_kind(
    export_name: &str,
    context: &RecoveredReturnTypeContext<'_, impl BuildHasher>,
) -> Option<RecoveredReturnValueKind> {
    let catalog = context.script_catalog?;
    let signatures = context.script_signatures?;
    let metadata = catalog.resolve_export_name(export_name)?;
    let signature = signatures
        .get(&metadata.packed_id)
        .unwrap_or(&metadata.signature);
    recovered_kind_from_return_type_name(&signature.return_type)
}

fn recovered_script_call_return_counts(
    export_name: &str,
    context: &RecoveredReturnTypeContext<'_, impl BuildHasher>,
) -> Option<super::ScriptReturnCounts> {
    let catalog = context.script_catalog?;
    let signatures = context.script_signatures?;
    let metadata = catalog.resolve_export_name(export_name)?;
    let signature = signatures
        .get(&metadata.packed_id)
        .unwrap_or(&metadata.signature);
    Some(super::ScriptReturnCounts {
        int: signature.return_count_int,
        obj: signature.return_count_obj,
        long: signature.return_count_long,
    })
}

fn recovered_kind_from_return_type_name(value: &str) -> Option<RecoveredReturnValueKind> {
    if recovered_return_type_contains(value, "string") {
        Some(RecoveredReturnValueKind::Object)
    } else if recovered_return_type_contains(value, "bigint") {
        Some(RecoveredReturnValueKind::Long)
    } else if recovered_return_type_contains(value, "number")
        || recovered_return_type_contains(value, "boolean")
    {
        Some(RecoveredReturnValueKind::Int)
    } else {
        None
    }
}

fn recovered_return_type_contains(value: &str, ty: &str) -> bool {
    value.split('|').any(|part| part.trim() == ty)
}

fn homogeneous_recovered_stack_return_kind(
    values: &[Expression],
    context: &RecoveredReturnTypeContext<'_, impl BuildHasher>,
) -> Option<RecoveredReturnValueKind> {
    let mut value_kinds = values
        .iter()
        .filter(|value| !is_recovered_void_script_call(value, context))
        .map(|value| infer_recovered_return_value_kind(value, context));
    let first = value_kinds.next()?;
    value_kinds.all(|kind| kind == first).then_some(first)
}

fn is_recovered_void_script_call(
    expr: &Expression,
    context: &RecoveredReturnTypeContext<'_, impl BuildHasher>,
) -> bool {
    let Expression::Call(call) = expr else {
        return false;
    };
    let Expression::Identifier(identifier) = &*call.callee else {
        return false;
    };
    let Some(catalog) = context.script_catalog else {
        return false;
    };
    let Some(signatures) = context.script_signatures else {
        return false;
    };
    let Some(metadata) = catalog.resolve_export_name(&identifier.name) else {
        return false;
    };
    let signature = signatures
        .get(&metadata.packed_id)
        .unwrap_or(&metadata.signature);
    signature.return_type.trim() == "void"
}

pub struct ExprRecovery<
    'a,
    VarHasher: std::hash::BuildHasher = std::collections::hash_map::RandomState,
    EnumHasher: std::hash::BuildHasher = std::collections::hash_map::RandomState,
    SignatureHasher: std::hash::BuildHasher = std::collections::hash_map::RandomState,
> {
    instructions: &'a [InstructionNode],
    stack: Vec<Expression>,
    locals: std::collections::HashMap<String, Expression>,
    var_names: &'a std::collections::HashMap<(VarDomain, u16), String, VarHasher>,
    /// Maps enum key values to qualified names (e.g. 0 → "`Enum_1234.ATTACK`").
    enum_value_names: &'a std::collections::HashMap<i32, String, EnumHasher>,
    script_catalog: &'a super::ScriptCatalog,
    script_signatures:
        &'a std::collections::HashMap<super::ScriptId, super::ScriptSignature, SignatureHasher>,
    build: u32,
    skip_count: usize,
    branch_targets: HashSet<usize>,
    return_pop_targets: HashSet<usize>,
}

fn collect_branch_targets(instructions: &[InstructionNode]) -> HashSet<usize> {
    let mut targets = HashSet::new();
    for instr in instructions {
        match &instr.operand {
            OperandNode::Branch(target) => {
                targets.insert(*target);
            }
            OperandNode::Switch(cases) => {
                targets.extend(cases.iter().map(|case| case.target));
            }
            _ => {}
        }
    }
    targets
}

impl<'a, VarHasher, EnumHasher, SignatureHasher>
    ExprRecovery<'a, VarHasher, EnumHasher, SignatureHasher>
where
    VarHasher: std::hash::BuildHasher,
    EnumHasher: std::hash::BuildHasher,
    SignatureHasher: std::hash::BuildHasher,
{
    pub fn new<ComponentHasher>(
        instructions: &'a [InstructionNode],
        var_names: &'a std::collections::HashMap<(VarDomain, u16), String, VarHasher>,
        component_names: &'a std::collections::HashMap<u32, String, ComponentHasher>,
        enum_value_names: &'a std::collections::HashMap<i32, String, EnumHasher>,
        script_catalog: &'a super::ScriptCatalog,
        script_signatures: &'a std::collections::HashMap<
            super::ScriptId,
            super::ScriptSignature,
            SignatureHasher,
        >,
    ) -> Self
    where
        ComponentHasher: std::hash::BuildHasher,
    {
        Self::new_for_build(
            instructions,
            var_names,
            component_names,
            enum_value_names,
            script_catalog,
            script_signatures,
            0,
        )
    }

    pub fn new_for_build<ComponentHasher>(
        instructions: &'a [InstructionNode],
        var_names: &'a std::collections::HashMap<(VarDomain, u16), String, VarHasher>,
        _component_names: &'a std::collections::HashMap<u32, String, ComponentHasher>,
        enum_value_names: &'a std::collections::HashMap<i32, String, EnumHasher>,
        script_catalog: &'a super::ScriptCatalog,
        script_signatures: &'a std::collections::HashMap<
            super::ScriptId,
            super::ScriptSignature,
            SignatureHasher,
        >,
        build: u32,
    ) -> Self
    where
        ComponentHasher: std::hash::BuildHasher,
    {
        Self {
            instructions,
            stack: Vec::new(),
            locals: std::collections::HashMap::new(),
            var_names,
            enum_value_names,
            script_catalog,
            script_signatures,
            build,
            skip_count: 0,
            branch_targets: collect_branch_targets(instructions),
            return_pop_targets: HashSet::new(),
        }
    }

    /// Process all instructions and return recovered statements.
    /// The result may have fewer entries than instructions (push/pop are
    /// folded into expressions).
    pub fn recover(mut self) -> Vec<Option<RecoveredStmt>> {
        let len = self.instructions.len();
        let mut stmts: Vec<Option<RecoveredStmt>> = vec![None; len];

        for (i, stmt_slot) in stmts.iter_mut().enumerate().take(len) {
            if self.skip_count > 0 {
                self.skip_count -= 1;
                continue;
            }
            let instr = &self.instructions[i];
            let effect = stack_effect(
                &instr.command,
                &instr.operand,
                self.build,
                self.script_catalog,
                self.script_signatures,
            );
            *stmt_slot = self.process_instruction(i, instr, &effect);
        }

        stmts
    }

    // Process is a large match with many `if let` arms that need early
    // return to avoid nested else branches. Converting to expression-
    // based returns would obscure the opcode dispatch pattern.
    #[allow(clippy::needless_return)]
    fn process_instruction(
        &mut self,
        index: usize,
        instr: &InstructionNode,
        effect: &StackEffect,
    ) -> Option<RecoveredStmt> {
        let cmd = instr.command.as_str();
        let op = &instr.operand;

        match cmd {
            // ── Push operations: build an expression and push onto stack ──
            "push_constant_int" => {
                if let OperandNode::Int(v) = op {
                    self.stack.push(Expression::Call(CallExpr {
                        callee: Box::new(Expression::Identifier(Identifier {
                            name: "intconst".to_string(),
                        })),
                        arguments: vec![Expression::NumberLiteral(NumberLiteral { value: *v })],
                    }));
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
                    self.stack.push(Expression::StringLiteral(StringLiteral {
                        value: s.clone(),
                    }));
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
                } else if let OperandNode::Long(v) = op {
                    self.stack.push(Expression::Call(CallExpr {
                        callee: Box::new(Expression::Identifier(Identifier {
                            name: "longconst".to_string(),
                        })),
                        arguments: vec![Expression::BigIntLiteral(super::ast::BigIntLiteral {
                            value: *v,
                        })],
                    }));
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
                    let name = varref_identifier(vr);
                    self.stack.push(Expression::Identifier(Identifier { name }));
                }
                None
            }
            "push_varbit" | "push_varclanbit" | "push_varclansettingbit" => {
                if let OperandNode::VarBitRef(vbr) = op {
                    let name = varbit_identifier(vbr);
                    self.stack.push(Expression::Identifier(Identifier { name }));
                }
                None
            }
            "pop_varbit" => {
                if let Some(stmt) = self.recover_stacked_assignment(index) {
                    return Some(stmt);
                }
                if let OperandNode::VarBitRef(vbr) = op {
                    let value = self.pop_or_unknown();
                    let name = varbit_identifier(vbr);
                    if self.should_preserve_stack_before_assignment_value(&value) {
                        return Some(self.preserve_stack_before_assignment(name, value));
                    }
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
                    let access = leave_index_array_call(*id, idx.clone());
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
                if let Some(stmt) = self.recover_stacked_assignment(index) {
                    return Some(stmt);
                }
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
                    let contains_multi_result = is_multi_result_access_value(&value)
                        || self.stack.iter().any(is_multi_result_access_value);
                    if !self.stack.is_empty() && !contains_multi_result {
                        return Some(self.preserve_stack_before_assignment(name, value));
                    }
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
                    if self.should_preserve_stack_before_assignment_value(&value) {
                        return Some(self.preserve_stack_before_assignment(name, value));
                    }
                    return Some(RecoveredStmt::Assignment {
                        target: name,
                        value,
                        var_type: "number".to_string(),
                    });
                }
                None
            }
            "pop_int_discard" | "pop_string_discard" | "pop_long_discard" => {
                let value = self.stack.pop().unwrap_or_else(|| discard_call(cmd));
                let value = if is_pop_call_expr(&value) {
                    discard_call(cmd)
                } else {
                    value
                };
                Some(RecoveredStmt::Expression(value))
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
                    let idx_str = if expression_uses_leave_index_array_call(&value, *id, &idx) {
                        "pop()".to_string()
                    } else {
                        expr_str(&idx)
                    };
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
                    let idx_str = if expression_uses_leave_index_array_call(&value, *id, &idx) {
                        "pop()".to_string()
                    } else {
                        expr_str(&idx)
                    };
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
                    let values = self.stack.clone();
                    if values.is_empty() {
                        self.stack.clear();
                        return Some(RecoveredStmt::Goto(*target));
                    }
                    self.stack.clear();
                    return Some(RecoveredStmt::GotoStack {
                        target: *target,
                        values,
                    });
                }
                None
            }
            "branch_not" | "long_branch_not" => {
                if let OperandNode::Branch(target) = op {
                    let right = self.pop_or_unknown();
                    let left = self.pop_or_unknown();
                    let left = self.preserve_stack_before_value(left);
                    self.stack.clear();
                    return Some(RecoveredStmt::BranchBinary {
                        op: BinaryOp::Ne,
                        left,
                        right,
                        target: *target,
                    });
                }
                None
            }
            "branch_if_true" => {
                if let OperandNode::Branch(target) = op {
                    let condition = self.pop_or_unknown();
                    let condition = self.preserve_stack_before_value(condition);
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
                    let condition = self.preserve_stack_before_value(condition);
                    self.stack.clear();
                    return Some(RecoveredStmt::Branch {
                        condition,
                        target: *target,
                        negated: true,
                    });
                }
                None
            }
            "branch_equals" | "long_branch_equals" => {
                if let OperandNode::Branch(target) = op {
                    let right = self.pop_or_unknown();
                    let left = self.pop_or_unknown();
                    let left = self.preserve_stack_before_value(left);
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
            "branch_less_than" | "long_branch_less_than" => {
                if let OperandNode::Branch(target) = op {
                    let right = self.pop_or_unknown();
                    let left = self.pop_or_unknown();
                    let left = self.preserve_stack_before_value(left);
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
            "branch_greater_than" | "long_branch_greater_than" => {
                if let OperandNode::Branch(target) = op {
                    let right = self.pop_or_unknown();
                    let left = self.pop_or_unknown();
                    let left = self.preserve_stack_before_value(left);
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
            "branch_less_than_or_equals" | "long_branch_less_than_or_equals" => {
                if let OperandNode::Branch(target) = op {
                    let right = self.pop_or_unknown();
                    let left = self.pop_or_unknown();
                    let left = self.preserve_stack_before_value(left);
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
            "branch_greater_than_or_equals" | "long_branch_greater_than_or_equals" => {
                if let OperandNode::Branch(target) = op {
                    let right = self.pop_or_unknown();
                    let left = self.pop_or_unknown();
                    let left = self.preserve_stack_before_value(left);
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
                    let discriminant = self.preserve_stack_before_value(discriminant);
                    let case_pairs: Vec<(i32, usize)> =
                        cases.iter().map(|c| (c.value, c.target)).collect();
                    return Some(RecoveredStmt::Switch {
                        discriminant,
                        cases: case_pairs,
                    });
                }
                None
            }
            "return" => {
                let val = match self.stack.len() {
                    0 if self.return_pop_targets.contains(&index) => Some(pop_call_expr()),
                    0 => None,
                    1 => self.stack.pop(),
                    _ => Some(Expression::Call(CallExpr {
                        callee: Box::new(Expression::Identifier(Identifier {
                            name: "stack".to_string(),
                        })),
                        arguments: std::mem::take(&mut self.stack),
                    })),
                };
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
                    let args = self.pop_arg_slots(signature.total_args());
                    let expr = Expression::Call(CallExpr {
                        callee: Box::new(Expression::Identifier(Identifier {
                            name: target.export_name.clone(),
                        })),
                        arguments: args,
                    });
                    if signature.return_type == "void" {
                        return Some(self.preserve_stack_before_statement(expr));
                    }
                    if self.next_return_is_branch_target(index) {
                        self.return_pop_targets.insert(index + 1);
                        return Some(RecoveredStmt::Expression(push_call_expr(expr)));
                    }
                    let return_count = signature
                        .total_returns()
                        .max(usize::from(signature.return_type != "void"));
                    if self.call_result_crosses_branch_target(index, return_count) {
                        return Some(self.materialize_call_result(expr));
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
            "unknown_command_28" => {
                let slot = self.pop_or_unknown();
                self.push_indexed_call_results(cmd, vec![slot], 2);
                None
            }
            "unknown_command_29" => {
                let row = self.pop_or_unknown();
                let menu = self.pop_or_unknown();
                self.push_indexed_call_results(cmd, vec![menu, row], 3);
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
            "store_lookup" => {
                let currency = self.pop_or_unknown();
                let position = self.pop_or_unknown();
                self.push_indexed_call_results(cmd, vec![position, currency], 13);
                None
            }
            "fullscreen_getmode" => {
                let index = self.pop_or_unknown();
                self.push_named_call_results(cmd, vec![index], &["width", "height"]);
                None
            }
            "cc_getcharposatindex" => {
                let index = self.pop_or_unknown();
                self.push_indexed_call_results(cmd, vec![index], 2);
                None
            }
            "if_getcharposatindex" => {
                let index = self.pop_or_unknown();
                let component = self.pop_or_unknown();
                self.push_indexed_call_results(cmd, vec![component, index], 2);
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
                let mut name = format!("UI.{}", sanitize_camel(&cmd[3..]));
                append_ui_operand_mode(&mut name, &mut arguments, op);
                let call = Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier { name })),
                    arguments,
                });
                return Some(self.preserve_stack_before_statement(call));
            }
            "cc_scriptqueue_add" | "if_scriptqueue_add" => {
                let has_component = cmd.starts_with("if_");
                let component = has_component.then(|| self.pop_or_unknown());
                let descriptor = self.pop_or_unknown();
                let callback = self.recover_callback_literal(cmd, &descriptor);
                let delay = self.pop_or_unknown();
                let mut arguments = Vec::with_capacity(2 + usize::from(has_component));
                arguments.push(delay);
                arguments.push(callback.unwrap_or(descriptor));
                if let Some(component) = component {
                    arguments.push(component);
                }
                let mut name = "UI.ScriptqueueAdd".to_string();
                append_ui_operand_mode(&mut name, &mut arguments, op);
                self.stack.push(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier { name })),
                    arguments,
                }));
                return None;
            }
            "cc_create" => {
                let child_id = self.pop_or_unknown();
                let component_type = self.pop_or_unknown();
                let parent = self.pop_or_unknown();
                let mut arguments = vec![parent, component_type, child_id];
                if let OperandNode::Byte(mode) = op
                    && *mode != 0
                {
                    arguments.push(Expression::NumberLiteral(NumberLiteral {
                        value: i32::from(*mode),
                    }));
                }
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "UI.create".to_string(),
                    })),
                    arguments,
                })));
            }
            "cc_delete" => {
                let mut name = "UI.delete".to_string();
                let mut arguments = Vec::new();
                append_ui_operand_mode(&mut name, &mut arguments, op);
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier { name })),
                    arguments,
                })));
            }
            "cc_deleteall" => {
                let component = self.pop_or_unknown();
                let mut name = "UI.deleteAll".to_string();
                let mut arguments = vec![component];
                append_ui_operand_mode(&mut name, &mut arguments, op);
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier { name })),
                    arguments,
                })));
            }
            "cc_find" => {
                let child_id = self.pop_or_unknown();
                let component = self.pop_or_unknown();
                let mut arguments = vec![component, child_id];
                if let OperandNode::Byte(mode) = op
                    && *mode != 0
                {
                    arguments.push(Expression::NumberLiteral(NumberLiteral {
                        value: i32::from(*mode),
                    }));
                }
                self.stack.push(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: "UI.find".to_string(),
                    })),
                    arguments,
                }));
                None
            }
            "if_find" => {
                let component = self.pop_or_unknown();
                let mode = match op {
                    OperandNode::Byte(mode) if *mode != 0 => Some(*mode),
                    _ => None,
                };
                let (name, arguments) = if let Some(mode) = mode {
                    (
                        "UI.findInterface",
                        vec![
                            component,
                            Expression::NumberLiteral(NumberLiteral {
                                value: i32::from(mode),
                            }),
                        ],
                    )
                } else {
                    ("UI.find", vec![component])
                };
                self.stack.push(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier {
                        name: name.to_string(),
                    })),
                    arguments,
                }));
                None
            }
            "cc_sendtofront" => {
                let mut name = "UI.sendToFront".to_string();
                let mut arguments = Vec::new();
                append_ui_operand_mode(&mut name, &mut arguments, op);
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier { name })),
                    arguments,
                })));
            }
            "cc_sendtoback" => {
                let mut name = "UI.sendToBack".to_string();
                let mut arguments = Vec::new();
                append_ui_operand_mode(&mut name, &mut arguments, op);
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier { name })),
                    arguments,
                })));
            }
            "if_sendtofront" => {
                let component = self.pop_or_unknown();
                let mut name = "UI.sendToFront".to_string();
                let mut arguments = vec![component];
                append_ui_operand_mode(&mut name, &mut arguments, op);
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier { name })),
                    arguments,
                })));
            }
            "if_sendtoback" => {
                let component = self.pop_or_unknown();
                let mut name = "UI.sendToBack".to_string();
                let mut arguments = vec![component];
                append_ui_operand_mode(&mut name, &mut arguments, op);
                return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                    callee: Box::new(Expression::Identifier(Identifier { name })),
                    arguments,
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
                return Some(self.ui_call("UI.setText", 1, op));
            }
            "cc_setgraphic" => {
                return Some(self.ui_call("UI.setGraphic", 1, op));
            }
            "cc_sethide" => {
                return Some(self.ui_call("UI.setHide", 1, op));
            }
            "cc_setcolour" => {
                return Some(self.ui_call("UI.setColour", 1, op));
            }
            "cc_setfill" => return Some(self.ui_call("UI.setFill", 1, op)),
            "cc_settrans" => return Some(self.ui_call("UI.setTrans", 1, op)),
            "cc_setlinewid" => return Some(self.ui_call("UI.setLineWid", 1, op)),
            "cc_setmodel" => return Some(self.ui_call("UI.setModel", 1, op)),
            "cc_setscrollpos" => return Some(self.ui_call("UI.setScrollPos", 2, op)),
            "cc_setscrollsize" => return Some(self.ui_call("UI.setScrollSize", 2, op)),
            "cc_setaspect" => return Some(self.ui_call("UI.setAspect", 2, op)),
            "cc_setposition" => return Some(self.ui_call("UI.setPosition", 4, op)),
            "cc_setsize" => {
                return Some(self.ui_call("UI.setSize", 4, op));
            }
            "cc_setmodelorigin" => return Some(self.ui_call("UI.setModelOrigin", 2, op)),
            "cc_setmodelangle" => return Some(self.ui_call("UI.setModelAngle", 6, op)),
            "cc_setmodelzoom" => return Some(self.ui_call("UI.setModelZoom", 1, op)),
            "cc_setmodelorthog" => return Some(self.ui_call("UI.setModelOrthog", 1, op)),
            "cc_setmodeltint" => return Some(self.ui_call("UI.setModelTint", 4, op)),
            "cc_setmodellighting" => return Some(self.ui_call("UI.setModelLighting", 10, op)),
            "cc_resetmodellighting" => return Some(self.ui_call("UI.resetModelLighting", 0, op)),
            "cc_settextfont" => return Some(self.ui_call("UI.setTextFont", 1, op)),
            "cc_settextalign" => return Some(self.ui_call("UI.setTextAlign", 3, op)),
            "cc_settextshadow" => return Some(self.ui_call("UI.setTextShadow", 1, op)),
            "cc_settextantimacro" => return Some(self.ui_call("UI.setTextAntiMacro", 1, op)),
            "cc_setoutline" => return Some(self.ui_call("UI.setOutline", 1, op)),
            "cc_setgraphicshadow" => return Some(self.ui_call("UI.setGraphicShadow", 1, op)),
            "cc_setclickmask" => return Some(self.ui_call("UI.setClickMask", 1, op)),
            "cc_setheld" => return Some(self.ui_call("UI.setHeld", 1, op)),
            "cc_setfontmono" => return Some(self.ui_call("UI.setFontMono", 1, op)),
            "cc_setparam" => return Some(self.ui_call("UI.setParam", 2, op)),
            "cc_setparam_int" => return Some(self.ui_call("UI.setParamInt", 2, op)),
            "cc_setparam_string" => return Some(self.ui_call("UI.setParamString", 2, op)),
            _ => {
                match categorize(cmd) {
                    OpcodeCategory::CC | OpcodeCategory::IF => {
                        let mut args: Vec<Expression> =
                            (0..effect.pops).map(|_| self.pop_or_unknown()).collect();
                        args.reverse();
                        let mut name = format!("UI.{}", sanitize_camel(&cmd[3..]));
                        append_ui_operand_mode(&mut name, &mut args, op);
                        let call = Expression::Call(CallExpr {
                            callee: Box::new(Expression::Identifier(Identifier { name })),
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
                        if !self.stack.is_empty() {
                            return Some(self.preserve_stack_before_statement(call));
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
                        // define_array carries array id as operand and consumes
                        // size from stack. Render id in function name so the
                        // TS form remains compact but still reversible.
                        if let OperandNode::Array(id) = op
                            && cmd == "define_array"
                        {
                            let size = self.pop_or_unknown();
                            return Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
                                callee: Box::new(Expression::Identifier(Identifier {
                                    name: format!("define_array_{id}"),
                                })),
                                arguments: vec![size],
                            })));
                        }
                        // Unknown: emit as call if it consumes or produces stack values
                        if effect.pops > 0 {
                            let mut args: Vec<Expression> =
                                (0..effect.pops).map(|_| self.pop_or_unknown()).collect();
                            args.reverse();
                            let mut name = sanitize_command(cmd);
                            append_opcode_operand_mode(&mut name, &mut args, op);
                            if effect.pushes > 0 {
                                let expr = Expression::Call(CallExpr {
                                    callee: Box::new(Expression::Identifier(Identifier { name })),
                                    arguments: args,
                                });
                                self.stack.push(expr);
                            } else {
                                let call = Expression::Call(CallExpr {
                                    callee: Box::new(Expression::Identifier(Identifier { name })),
                                    arguments: args,
                                });
                                if should_preserve_stack_before_statement(cmd) {
                                    return Some(self.preserve_stack_before_statement(call));
                                }
                                return Some(RecoveredStmt::Expression(call));
                            }
                        } else if effect.pushes > 0 {
                            let mut name = sanitize_command(cmd);
                            let mut arguments = Vec::new();
                            append_opcode_operand_mode(&mut name, &mut arguments, op);
                            let call = Expression::Call(CallExpr {
                                callee: Box::new(Expression::Identifier(Identifier { name })),
                                arguments,
                            });
                            let call = if should_preserve_stack_before_value(cmd) {
                                self.preserve_stack_before_value(call)
                            } else {
                                call
                            };
                            self.stack.push(call);
                        } else {
                            let mut name = sanitize_command(cmd);
                            let mut arguments = Vec::new();
                            append_opcode_operand_mode(&mut name, &mut arguments, op);
                            let call = Expression::Call(CallExpr {
                                callee: Box::new(Expression::Identifier(Identifier { name })),
                                arguments,
                            });
                            if should_preserve_stack_before_statement(cmd) {
                                return Some(self.preserve_stack_before_statement(call));
                            }
                            return Some(RecoveredStmt::Expression(call));
                        }
                    }
                }
                None
            }
        }
    }

    fn next_return_is_branch_target(&self, index: usize) -> bool {
        let next = index + 1;
        self.branch_targets.contains(&next)
            && self
                .instructions
                .get(next)
                .is_some_and(|instr| instr.command == "return")
    }

    fn preserve_stack_before_statement(&mut self, statement: Expression) -> RecoveredStmt {
        if self.stack.is_empty() {
            return RecoveredStmt::Expression(statement);
        }
        let mut arguments = std::mem::take(&mut self.stack);
        self.stack
            .extend((0..arguments.len()).map(|_| pop_call_expr()));
        arguments.push(statement);
        RecoveredStmt::Expression(Expression::Call(CallExpr {
            callee: Box::new(Expression::Identifier(Identifier {
                name: "stackpush_then".to_string(),
            })),
            arguments,
        }))
    }

    fn preserve_stack_before_value(&mut self, value: Expression) -> Expression {
        if self.stack.is_empty() {
            return value;
        }
        let mut arguments = std::mem::take(&mut self.stack);
        self.stack
            .extend((0..arguments.len()).map(|_| pop_call_expr()));
        arguments.push(value);
        Expression::Call(CallExpr {
            callee: Box::new(Expression::Identifier(Identifier {
                name: "stackpush_then".to_string(),
            })),
            arguments,
        })
    }

    fn materialize_call_result(&mut self, call: Expression) -> RecoveredStmt {
        let push_call = push_call_expr(call);
        if self.stack.is_empty() {
            self.stack.push(pop_call_expr());
            return RecoveredStmt::Expression(push_call);
        }

        let mut arguments = std::mem::take(&mut self.stack);
        self.stack
            .extend((0..=arguments.len()).map(|_| pop_call_expr()));
        arguments.push(push_call);
        RecoveredStmt::Expression(Expression::Call(CallExpr {
            callee: Box::new(Expression::Identifier(Identifier {
                name: "stackpush_then".to_string(),
            })),
            arguments,
        }))
    }

    fn preserve_stack_before_assignment(
        &mut self,
        target: String,
        value: Expression,
    ) -> RecoveredStmt {
        let assignment = Expression::Call(CallExpr {
            callee: Box::new(Expression::Identifier(Identifier {
                name: "stackassign_1".to_string(),
            })),
            arguments: vec![
                Expression::StringLiteral(StringLiteral { value: target }),
                value,
            ],
        });
        self.preserve_stack_before_statement(assignment)
    }

    fn should_preserve_stack_before_assignment_value(&self, value: &Expression) -> bool {
        !self.stack.is_empty()
            && !is_multi_result_access_value(value)
            && !self.stack.iter().any(is_multi_result_access_value)
    }

    fn recover_stacked_assignment(&mut self, index: usize) -> Option<RecoveredStmt> {
        let mut targets = Vec::new();
        for instr in &self.instructions[index..] {
            let Some(target) = stacked_pop_target(instr) else {
                break;
            };
            targets.push(target);
            if targets.len() >= self.stack.len() {
                break;
            }
        }
        if targets.len() < 2 || self.stack.len() < targets.len() {
            return None;
        }

        let values = self.stack.split_off(self.stack.len() - targets.len());
        if values.iter().any(is_multi_result_access_value) {
            self.stack.extend(values);
            return None;
        }
        for (target, value) in targets.iter().zip(values.iter().rev()) {
            self.locals.insert(target.clone(), value.clone());
        }

        let mut arguments = Vec::with_capacity(targets.len() + values.len());
        arguments.extend(targets.iter().map(|target| {
            Expression::StringLiteral(StringLiteral {
                value: target.clone(),
            })
        }));
        arguments.extend(values);

        self.skip_count = targets.len() - 1;
        Some(RecoveredStmt::Expression(Expression::Call(CallExpr {
            callee: Box::new(Expression::Identifier(Identifier {
                name: format!("stackassign_{}", targets.len()),
            })),
            arguments,
        })))
    }

    fn pop_expr(&mut self) -> Option<Expression> {
        self.stack.pop()
    }

    fn pop_or_unknown(&mut self) -> Expression {
        self.stack.pop().unwrap_or_else(pop_call_expr)
    }

    fn pop_args(&mut self, count: usize) -> Vec<Expression> {
        let mut args = Vec::with_capacity(count);
        for _ in 0..count {
            args.push(self.pop_or_unknown());
        }
        args.reverse();
        args
    }

    fn pop_arg_slots(&mut self, count: usize) -> Vec<Expression> {
        let mut args = Vec::new();
        let mut slots = 0;
        while slots < count {
            let expr = self.pop_or_unknown();
            slots += self.expression_stack_width(&expr);
            args.push(expr);
        }
        args.reverse();
        args
    }

    fn expression_stack_width(&self, expr: &Expression) -> usize {
        let Expression::Call(call) = expr else {
            return 1;
        };
        let Expression::Identifier(identifier) = &*call.callee else {
            return 1;
        };
        let Some(metadata) = self.script_catalog.resolve_export_name(&identifier.name) else {
            return 1;
        };
        let signature = self
            .script_signatures
            .get(&metadata.packed_id)
            .unwrap_or(&metadata.signature);
        signature.total_returns().max(1)
    }

    fn call_result_crosses_branch_target(&self, index: usize, return_count: usize) -> bool {
        if return_count != 1 {
            return false;
        }

        let mut values_above_call = 0usize;
        for next in index + 1..self.instructions.len() {
            if self.branch_targets.contains(&next) {
                return true;
            }

            let instruction = &self.instructions[next];
            let effect = stack_effect(
                &instruction.command,
                &instruction.operand,
                self.build,
                self.script_catalog,
                self.script_signatures,
            );
            if effect.pops > values_above_call {
                return false;
            }
            values_above_call = values_above_call - effect.pops + effect.pushes;
        }

        false
    }

    fn ui_call(&mut self, name: &str, arg_count: usize, op: &OperandNode) -> RecoveredStmt {
        let mut name = name.to_string();
        let mut arguments = self.pop_args(arg_count);
        append_ui_operand_mode(&mut name, &mut arguments, op);
        RecoveredStmt::Expression(Expression::Call(CallExpr {
            callee: Box::new(Expression::Identifier(Identifier { name })),
            arguments,
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
        self.push_multi_slot_expr(&call, count);
    }

    fn push_multi_slot_expr(&mut self, expr: &Expression, count: usize) {
        for index in 0..count {
            self.stack
                .push(Expression::ArrayAccess(super::ast::ArrayAccess {
                    array: Box::new(expr.clone()),
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
            let count = self
                .stack
                .last()
                .and_then(|expr| self.literal_usize_expr(expr))
                .filter(|count| *count < self.stack.len())
                .filter(|count| *count <= MAX_CALLBACK_WATCHERS)?;
            self.stack.pop();
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

    fn literal_usize_expr(&self, expr: &Expression) -> Option<usize> {
        usize::try_from(self.literal_i32_expr(expr)?).ok()
    }

    fn literal_i32_expr(&self, expr: &Expression) -> Option<i32> {
        literal_i32(expr).or_else(|| enum_named_literal_i32(expr, self.enum_value_names))
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

fn stacked_pop_target(instr: &InstructionNode) -> Option<String> {
    match (instr.command.as_str(), &instr.operand) {
        ("pop_int_local", OperandNode::Local(idx)) => Some(format!("local_int_{idx}")),
        ("pop_string_local", OperandNode::Local(idx)) => Some(format!("local_obj_{idx}")),
        ("pop_long_local", OperandNode::Local(idx)) => Some(format!("local_long_{idx}")),
        ("pop_varbit", OperandNode::VarBitRef(varbit)) => Some(varbit_identifier(varbit)),
        _ => None,
    }
}

fn is_multi_result_access_value(value: &Expression) -> bool {
    match value {
        Expression::PropertyAccess(access) => matches!(&*access.object, Expression::Call(_)),
        Expression::ArrayAccess(access) => matches!(&*access.array, Expression::Call(_)),
        _ => false,
    }
}

fn discard_call(cmd: &str) -> Expression {
    let name = match cmd {
        "pop_string_discard" => "popstringdiscard",
        "pop_long_discard" => "poplongdiscard",
        _ => "popintdiscard",
    };
    Expression::Call(CallExpr {
        callee: Box::new(Expression::Identifier(Identifier {
            name: name.to_string(),
        })),
        arguments: vec![],
    })
}

fn pop_call_expr() -> Expression {
    Expression::Call(CallExpr {
        callee: Box::new(Expression::Identifier(Identifier {
            name: "pop".to_string(),
        })),
        arguments: vec![],
    })
}

fn leave_index_array_call(array_id: i32, index: Expression) -> Expression {
    Expression::Call(CallExpr {
        callee: Box::new(Expression::Identifier(Identifier {
            name: format!("push_array_int_leave_index_on_stack_{array_id}"),
        })),
        arguments: vec![index],
    })
}

fn push_call_expr(value: Expression) -> Expression {
    Expression::Call(CallExpr {
        callee: Box::new(Expression::Identifier(Identifier {
            name: "push".to_string(),
        })),
        arguments: vec![value],
    })
}

fn is_pop_call_expr(value: &Expression) -> bool {
    match value {
        Expression::Call(call) if call.arguments.is_empty() => {
            matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "pop")
        }
        Expression::PopOperation(_) => true,
        _ => false,
    }
}

fn expression_uses_leave_index_array_call(
    value: &Expression,
    array_id: i32,
    index: &Expression,
) -> bool {
    match value {
        Expression::Call(call) => {
            matches!(
                &*call.callee,
                Expression::Identifier(identifier)
                    if identifier.name == format!("push_array_int_leave_index_on_stack_{array_id}")
            ) && call
                .arguments
                .first()
                .is_some_and(|argument| expr_str(argument) == expr_str(index))
                || call.arguments.iter().any(|argument| {
                    expression_uses_leave_index_array_call(argument, array_id, index)
                })
        }
        Expression::ArrayAccess(access) => {
            expression_uses_leave_index_array_call(&access.array, array_id, index)
                || expression_uses_leave_index_array_call(&access.index, array_id, index)
        }
        Expression::PropertyAccess(access) => {
            expression_uses_leave_index_array_call(&access.object, array_id, index)
        }
        Expression::BinaryOperation(binary) => {
            expression_uses_leave_index_array_call(&binary.left, array_id, index)
                || expression_uses_leave_index_array_call(&binary.right, array_id, index)
        }
        Expression::UnaryOperation(unary) => {
            expression_uses_leave_index_array_call(&unary.operand, array_id, index)
        }
        Expression::PushOperation(push) => {
            expression_uses_leave_index_array_call(&push.value, array_id, index)
        }
        Expression::PopOperation(pop) => pop
            .target
            .as_ref()
            .is_some_and(|target| expression_uses_leave_index_array_call(target, array_id, index)),
        Expression::CallbackLiteral(callback) => callback
            .arguments
            .iter()
            .any(|argument| expression_uses_leave_index_array_call(argument, array_id, index)),
        Expression::NumberLiteral(_)
        | Expression::BigIntLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::BooleanLiteral(_)
        | Expression::Identifier(_)
        | Expression::GotoExpr(_) => false,
    }
}

fn should_preserve_stack_before_statement(command: &str) -> bool {
    matches!(
        command,
        "autosetup_setultra"
            | "cam2_resetsnapdistances"
            | "cam2_setpositionangularinterpolation"
            | "lobby_enterlobby_social_network"
    )
}

fn should_preserve_stack_before_value(command: &str) -> bool {
    matches!(command, "unknown_command_20")
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
            let name = varref_identifier(vr);
            Expression::Identifier(Identifier { name })
        }
        OperandNode::VarBitRef(vbr) => {
            let name = varbit_identifier(vbr);
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

fn is_interface_hook_opcode(cmd: &str) -> bool {
    (cmd.starts_with("if_") || cmd.starts_with("cc_")) && cmd.contains("_seton")
}

fn append_ui_operand_mode(name: &mut String, arguments: &mut Vec<Expression>, op: &OperandNode) {
    if let OperandNode::Byte(mode) = op
        && *mode != 0
    {
        name.push_str("WithMode");
        arguments.push(Expression::NumberLiteral(NumberLiteral {
            value: i32::from(*mode),
        }));
    }
}

fn append_opcode_operand_mode(
    name: &mut String,
    arguments: &mut Vec<Expression>,
    op: &OperandNode,
) {
    if let OperandNode::Byte(mode) = op
        && *mode != 0
    {
        name.push_str("WithMode");
        arguments.push(Expression::NumberLiteral(NumberLiteral {
            value: i32::from(*mode),
        }));
    }
}

fn varref_identifier(vr: &super::ast::VarRefNode) -> String {
    let base = vr
        .name
        .clone()
        .unwrap_or_else(|| format!("var_{}:{}", vr.domain.as_label(), vr.id));
    if vr.is_transmog {
        format!("{base}_transmog")
    } else {
        base
    }
}

fn varbit_identifier(vbr: &super::ast::VarBitRefNode) -> String {
    let base = vbr
        .name
        .clone()
        .unwrap_or_else(|| format!("varbit_{}", vbr.id));
    if vbr.is_transmog {
        format!("{base}_transmog")
    } else {
        base
    }
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

fn enum_named_literal_i32<EnumHasher>(
    expr: &Expression,
    enum_value_names: &std::collections::HashMap<i32, String, EnumHasher>,
) -> Option<i32>
where
    EnumHasher: std::hash::BuildHasher,
{
    let Expression::PropertyAccess(property) = expr else {
        return None;
    };
    let Expression::Identifier(object) = property.object.as_ref() else {
        return None;
    };

    enum_value_names
        .iter()
        .find_map(|(value, qualified)| match qualified.split_once('.') {
            Some((enum_name, variant))
                if enum_name == object.name && variant == property.property =>
            {
                Some(*value)
            }
            _ => None,
        })
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

fn hook_watcher_name<VarHasher>(
    cmd: &str,
    expr: &Expression,
    var_names: &std::collections::HashMap<(VarDomain, u16), String, VarHasher>,
) -> String
where
    VarHasher: std::hash::BuildHasher,
{
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
    use super::{
        ExprRecovery, RecoveredStmt, detect_return_type_from_recovered,
        detect_return_type_from_recovered_with_signatures, is_pop_call_expr, opcode_stack_effect,
        opcode_stack_effect_for_build,
    };
    use crate::transpile::ast::{
        BigIntLiteral, BinaryOp, CallExpr, Expression, Identifier, InstructionNode, NumberLiteral,
        OperandNode, StringLiteral, SwitchCase, VarBitId, VarBitRefNode, VarId, VarRefNode,
    };
    use crate::transpile::{
        ScriptCatalog, ScriptGroupId, ScriptId, ScriptKind, ScriptMetadata, ScriptSignature,
    };
    use crate::vars::VarDomain;
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
    fn define_array_consumes_size_argument_in_recovered_call() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(4),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "define_array".to_string(),
                operand: OperandNode::Array(0),
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
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "define_array_0")
                    && call.arguments.len() == 1
        ));
    }

    #[test]
    fn leave_index_array_read_recovers_explicit_pseudo_call() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(12),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_array_int_leave_index_on_stack".to_string(),
                operand: OperandNode::Array(2),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(27),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "add".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "pop_array_int".to_string(),
                operand: OperandNode::Array(2),
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
            Some(RecoveredStmt::Assignment {
                target,
                value: Expression::BinaryOperation(binary),
                ..
            }) if target == "array_2[pop()]"
                && matches!(
                    &*binary.left,
                    Expression::Call(call)
                        if matches!(
                            &*call.callee,
                            Expression::Identifier(identifier)
                                if identifier.name == "push_array_int_leave_index_on_stack_2"
                        )
                )
        ));
    }

    #[test]
    fn branch_not_recovers_binary_not_equals_condition() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(1),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(-1),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "branch_not".to_string(),
                operand: OperandNode::Branch(7),
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
            Some(RecoveredStmt::BranchBinary {
                op: BinaryOp::Ne,
                left,
                right,
                target: 7,
            }) if matches!(left, Expression::Identifier(identifier) if identifier.name == "local_int_1")
                && matches!(right, Expression::NumberLiteral(number) if number.value == -1)
        ));
    }

    #[test]
    fn branch_preserves_pending_stack_values_for_stack_goto() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(18),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(1),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "branch".to_string(),
                operand: OperandNode::Branch(9),
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

        match &recovered[2] {
            Some(RecoveredStmt::GotoStack { target, values }) => {
                assert_eq!(*target, 9);
                assert_eq!(values.len(), 2);
                assert!(
                    matches!(&values[0], Expression::NumberLiteral(value) if value.value == 18)
                );
                assert!(matches!(&values[1], Expression::NumberLiteral(value) if value.value == 1));
            }
            other => panic!("expected stack-preserving goto, got {other:?}"),
        }
    }

    #[test]
    fn switch_preserves_pending_stack_values_before_discriminant() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "unknown_command_20".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(2),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "switch".to_string(),
                operand: OperandNode::Switch(vec![SwitchCase {
                    value: 2,
                    target: 4,
                }]),
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
            &recovered[2],
            Some(RecoveredStmt::Switch {
                discriminant: Expression::Call(call),
                ..
            }) if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "stackpush_then")
                && call.arguments.len() == 2
        ));
        assert!(matches!(
            &recovered[3],
            Some(RecoveredStmt::Return(Some(Expression::Call(call))))
                if is_pop_call_expr(&Expression::Call(call.clone()))
        ));
    }

    #[test]
    fn void_gosub_preserves_existing_stack_values_before_call() {
        let value_signature = ScriptSignature {
            arg_count_int: 1,
            arg_count_obj: 0,
            arg_count_long: 0,
            return_count_int: 1,
            return_count_obj: 0,
            return_count_long: 0,
            return_type: "number".to_string(),
        };
        let void_signature = ScriptSignature {
            arg_count_int: 1,
            arg_count_obj: 0,
            arg_count_long: 0,
            return_count_int: 0,
            return_count_obj: 0,
            return_count_long: 0,
            return_type: "void".to_string(),
        };
        let mut catalog = ScriptCatalog::default();
        catalog.insert(ScriptMetadata {
            packed_id: ScriptId(1),
            group_id: ScriptGroupId(1),
            file_id: 0,
            kind: ScriptKind::Unknown,
            raw_name: None,
            short_name: "value_helper".to_string(),
            export_name: "value_helper".to_string(),
            module_name: "value_helper".to_string(),
            signature: value_signature.clone(),
        });
        catalog.insert(ScriptMetadata {
            packed_id: ScriptId(2),
            group_id: ScriptGroupId(2),
            file_id: 0,
            kind: ScriptKind::Unknown,
            raw_name: None,
            short_name: "void_helper".to_string(),
            export_name: "void_helper".to_string(),
            module_name: "void_helper".to_string(),
            signature: void_signature.clone(),
        });
        let signatures = HashMap::from([
            (ScriptId(1), value_signature),
            (ScriptId(2), void_signature),
        ]);
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(7),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "gosub_with_params".to_string(),
                operand: OperandNode::Script(1),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(8),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "gosub_with_params".to_string(),
                operand: OperandNode::Script(2),
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
            &catalog,
            &signatures,
        )
        .recover();

        assert!(matches!(
            &recovered[3],
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "stackpush_then")
                    && call.arguments.len() == 2
        ));
        assert!(matches!(
            &recovered[4],
            Some(RecoveredStmt::Return(Some(Expression::Call(call))))
                if is_pop_call_expr(&Expression::Call(call.clone()))
        ));
    }

    #[test]
    fn generic_ui_store_preserves_pending_legacy_value_call() {
        let legacy_value_signature = ScriptSignature {
            arg_count_int: 2,
            arg_count_obj: 0,
            arg_count_long: 0,
            return_count_int: 0,
            return_count_obj: 0,
            return_count_long: 0,
            return_type: "number".to_string(),
        };
        let mut catalog = ScriptCatalog::default();
        catalog.insert(ScriptMetadata {
            packed_id: ScriptId(1),
            group_id: ScriptGroupId(1),
            file_id: 0,
            kind: ScriptKind::Unknown,
            raw_name: None,
            short_name: "legacy_value".to_string(),
            export_name: "legacy_value".to_string(),
            module_name: "legacy_value".to_string(),
            signature: legacy_value_signature.clone(),
        });
        let signatures = HashMap::from([(ScriptId(1), legacy_value_signature)]);
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(1),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(0),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "gosub_with_params".to_string(),
                operand: OperandNode::Script(1),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(0),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(1),
            },
            InstructionNode {
                index: 5,
                opcode: 0,
                command: "if_sethide".to_string(),
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
            &catalog,
            &signatures,
        )
        .recover();

        assert!(matches!(
            &recovered[5],
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "stackpush_then")
                    && matches!(call.arguments.first(), Some(Expression::Call(pending))
                        if matches!(&*pending.callee, Expression::Identifier(identifier) if identifier.name == "legacy_value"))
        ));
    }

    #[test]
    fn value_gosub_before_targeted_return_recovers_push_and_return_pop() {
        let signature = ScriptSignature {
            arg_count_int: 1,
            arg_count_obj: 0,
            arg_count_long: 0,
            return_count_int: 1,
            return_count_obj: 0,
            return_count_long: 0,
            return_type: "number".to_string(),
        };
        let mut catalog = ScriptCatalog::default();
        catalog.insert(ScriptMetadata {
            packed_id: ScriptId(1),
            group_id: ScriptGroupId(1),
            file_id: 0,
            kind: ScriptKind::Unknown,
            raw_name: None,
            short_name: "value_helper".to_string(),
            export_name: "value_helper".to_string(),
            module_name: "value_helper".to_string(),
            signature: signature.clone(),
        });
        let signatures = HashMap::from([(ScriptId(1), signature)]);
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "branch".to_string(),
                operand: OperandNode::Branch(3),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(7),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "gosub_with_params".to_string(),
                operand: OperandNode::Script(1),
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
            &catalog,
            &signatures,
        )
        .recover();

        assert!(matches!(
            &recovered[2],
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "push")
        ));
        assert!(matches!(
            &recovered[3],
            Some(RecoveredStmt::Return(Some(Expression::Call(call))))
                if is_pop_call_expr(&Expression::Call(call.clone()))
        ));
    }

    #[test]
    fn multi_return_gosub_supplies_multiple_later_arguments() {
        let producer_signature = ScriptSignature {
            arg_count_int: 1,
            arg_count_obj: 0,
            arg_count_long: 0,
            return_count_int: 3,
            return_count_obj: 0,
            return_count_long: 0,
            return_type: "number".to_string(),
        };
        let consumer_signature = ScriptSignature {
            arg_count_int: 5,
            arg_count_obj: 0,
            arg_count_long: 0,
            return_count_int: 0,
            return_count_obj: 1,
            return_count_long: 0,
            return_type: "string".to_string(),
        };
        let mut catalog = ScriptCatalog::default();
        catalog.insert(ScriptMetadata {
            packed_id: ScriptId(1),
            group_id: ScriptGroupId(1),
            file_id: 0,
            kind: ScriptKind::Unknown,
            raw_name: None,
            short_name: "producer".to_string(),
            export_name: "producer".to_string(),
            module_name: "producer".to_string(),
            signature: producer_signature.clone(),
        });
        catalog.insert(ScriptMetadata {
            packed_id: ScriptId(2),
            group_id: ScriptGroupId(2),
            file_id: 0,
            kind: ScriptKind::Unknown,
            raw_name: None,
            short_name: "consumer".to_string(),
            export_name: "consumer".to_string(),
            module_name: "consumer".to_string(),
            signature: consumer_signature.clone(),
        });
        let signatures = HashMap::from([
            (ScriptId(1), producer_signature),
            (ScriptId(2), consumer_signature),
        ]);
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::String("Score: ".to_string()),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(0),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "gosub_with_params".to_string(),
                operand: OperandNode::Script(1),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(1),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(0),
            },
            InstructionNode {
                index: 5,
                opcode: 0,
                command: "gosub_with_params".to_string(),
                operand: OperandNode::Script(2),
            },
            InstructionNode {
                index: 6,
                opcode: 0,
                command: "join_string".to_string(),
                operand: OperandNode::Count(2),
            },
            InstructionNode {
                index: 7,
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
            &catalog,
            &signatures,
        )
        .recover();

        let Some(RecoveredStmt::Return(Some(Expression::Call(concat)))) = &recovered[7] else {
            panic!("expected concat return");
        };
        assert_eq!(concat.arguments.len(), 2);
        let Expression::Call(consumer) = &concat.arguments[1] else {
            panic!("expected consumer call");
        };
        assert_eq!(consumer.arguments.len(), 3);
        assert!(matches!(
            &consumer.arguments[0],
            Expression::Call(call)
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "producer")
        ));
    }

    #[test]
    fn local_assignment_preserves_pending_stack_values_before_store() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(1),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(5),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(3),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(1),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "add".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 5,
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

        assert!(matches!(
            &recovered[5],
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "stackpush_then")
                    && call.arguments.len() == 3
                    && matches!(call.arguments.last(), Some(Expression::Call(inner))
                        if matches!(&*inner.callee, Expression::Identifier(identifier) if identifier.name == "stackassign_1"))
        ));
    }

    #[test]
    fn var_assignment_preserves_pending_stack_values_before_store() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(0),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(-1),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "pop_var".to_string(),
                operand: OperandNode::VarRef(VarRefNode {
                    domain: VarDomain::Client,
                    id: VarId(6908),
                    name: Some("varclient_6908".to_string()),
                    is_transmog: false,
                }),
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
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "stackpush_then")
                    && call.arguments.len() == 2
                    && matches!(call.arguments.last(), Some(Expression::Call(inner))
                        if matches!(&*inner.callee, Expression::Identifier(identifier) if identifier.name == "stackassign_1")
                            && matches!(inner.arguments.first(), Some(Expression::StringLiteral(StringLiteral { value })) if value == "varclient_6908"))
        ));
    }

    #[test]
    fn long_branch_recovers_binary_condition() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_long_local".to_string(),
                operand: OperandNode::Local(0),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Long(1),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "long_branch_equals".to_string(),
                operand: OperandNode::Branch(4),
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
            Some(RecoveredStmt::BranchBinary {
                op: BinaryOp::Eq,
                left,
                right,
                target: 4,
            }) if matches!(left, Expression::Identifier(identifier) if identifier.name == "local_long_0")
                && matches!(right, Expression::Call(call)
                    if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "longconst")
                        && matches!(call.arguments.as_slice(), [Expression::BigIntLiteral(BigIntLiteral { value: 1 })]))
        ));
    }

    #[test]
    fn consecutive_local_pops_recover_as_stackassign() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(20),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(40),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(3),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(2),
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
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "stackassign_2")
                    && matches!(call.arguments.as_slice(), [
                        Expression::StringLiteral(StringLiteral { value: target_0 }),
                        Expression::StringLiteral(StringLiteral { value: target_1 }),
                        Expression::NumberLiteral(NumberLiteral { value: 20 }),
                        Expression::NumberLiteral(NumberLiteral { value: 40 }),
                    ] if target_0 == "local_int_3" && target_1 == "local_int_2")
        ));
        assert!(recovered[3].is_none());
    }

    #[test]
    fn consecutive_varbit_pops_recover_as_stackassign() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(20),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(40),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "pop_varbit".to_string(),
                operand: OperandNode::VarBitRef(VarBitRefNode {
                    id: VarBitId(7058),
                    name: Some("varplayerbit_7058".to_string()),
                    is_transmog: false,
                }),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "pop_varbit".to_string(),
                operand: OperandNode::VarBitRef(VarBitRefNode {
                    id: VarBitId(7057),
                    name: Some("varplayerbit_7057".to_string()),
                    is_transmog: false,
                }),
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
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "stackassign_2")
                    && matches!(call.arguments.as_slice(), [
                        Expression::StringLiteral(StringLiteral { value: target_0 }),
                        Expression::StringLiteral(StringLiteral { value: target_1 }),
                        Expression::NumberLiteral(NumberLiteral { value: 20 }),
                        Expression::NumberLiteral(NumberLiteral { value: 40 }),
                    ] if target_0 == "varplayerbit_7058" && target_1 == "varplayerbit_7057")
        ));
        assert!(recovered[3].is_none());
    }

    #[test]
    fn push_constant_int_recovers_legacy_intconst_call() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(16),
            },
            InstructionNode {
                index: 1,
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
            &recovered[1],
            Some(RecoveredStmt::Return(Some(Expression::Call(call))))
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "intconst")
                    && matches!(&call.arguments[0], Expression::NumberLiteral(number) if number.value == 16)
        ));
    }

    #[test]
    fn typed_long_constant_recovers_longconst_call() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Long(0),
            },
            InstructionNode {
                index: 1,
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
            &recovered[1],
            Some(RecoveredStmt::Return(Some(Expression::Call(call))))
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "longconst")
                    && matches!(&call.arguments[0], Expression::BigIntLiteral(bigint) if bigint.value == 0)
        ));
    }

    #[test]
    fn command_signatures_supply_new_ui_stack_effects() {
        let effect = opcode_stack_effect("cc_input_setemptytext")
            .expect("interface component signature should provide stack effect");
        let if_set2dangle = opcode_stack_effect("if_set2dangle")
            .expect("interface component signature should override handler helper effect");
        let if_setnpcmodel = opcode_stack_effect("if_setnpcmodel")
            .expect("interface component signature should override handler helper effect");

        assert_eq!(
            (
                effect.int_pops,
                effect.obj_pops,
                effect.long_pops,
                effect.int_pushes,
                effect.obj_pushes,
                effect.long_pushes,
            ),
            (2, 1, 0, 0, 0, 0)
        );
        assert_eq!(if_set2dangle.total_pops(), 2);
        assert_eq!(if_set2dangle.total_pushes(), 0);
        assert_eq!(if_setnpcmodel.total_pops(), 2);
        assert_eq!(if_setnpcmodel.total_pushes(), 0);
    }

    #[test]
    fn command_signatures_supply_config_core_and_quest_stack_effects() {
        let client_option = opcode_stack_effect("clientoption_get")
            .expect("detail option signature should provide stack effect");
        let quest_name =
            opcode_stack_effect("quest_getname").expect("quest signature should provide effect");
        let var_ref =
            opcode_stack_effect("var_reference_get").expect("core signature should provide effect");
        let achievement_name = opcode_stack_effect("achievement_getname")
            .expect("achievement signature should provide effect");

        assert_eq!(
            (
                client_option.int_pops,
                client_option.obj_pops,
                client_option.int_pushes,
                client_option.obj_pushes,
            ),
            (1, 0, 1, 0)
        );
        assert_eq!(
            (
                quest_name.int_pops,
                quest_name.obj_pops,
                quest_name.int_pushes,
                quest_name.obj_pushes,
            ),
            (1, 0, 0, 1)
        );
        assert_eq!(
            (
                var_ref.int_pops,
                var_ref.obj_pops,
                var_ref.int_pushes,
                var_ref.obj_pushes,
            ),
            (1, 0, 1, 0)
        );
        assert_eq!(
            (
                achievement_name.int_pops,
                achievement_name.obj_pops,
                achievement_name.int_pushes,
                achievement_name.obj_pushes,
            ),
            (1, 0, 0, 1)
        );
    }

    #[test]
    fn command_signatures_supply_file_system_and_wikisync_stack_effects() {
        let file_system =
            opcode_stack_effect("unknown_command_38").expect("file-system signature effect");
        let wikisync = opcode_stack_effect("unknown_command_109")
            .expect("wikisync signature should provide effect");
        let minimenu_filter =
            opcode_stack_effect("unknown_command_36").expect("mini menu signature effect");
        let interface_target =
            opcode_stack_effect("unknown_command_26").expect("interface misc signature effect");
        let interface_drag =
            opcode_stack_effect("unknown_command_40").expect("interface misc signature effect");

        assert_eq!(
            (
                file_system.int_pops,
                file_system.obj_pops,
                file_system.int_pushes,
                file_system.obj_pushes,
            ),
            (1, 0, 0, 0)
        );
        assert_eq!(
            (
                wikisync.int_pops,
                wikisync.obj_pops,
                wikisync.int_pushes,
                wikisync.obj_pushes,
            ),
            (0, 2, 0, 0)
        );
        assert_eq!(
            (
                minimenu_filter.int_pops,
                minimenu_filter.obj_pops,
                minimenu_filter.int_pushes,
                minimenu_filter.obj_pushes,
            ),
            (3, 0, 0, 0)
        );
        assert_eq!(
            (
                interface_target.int_pops,
                interface_target.obj_pops,
                interface_target.int_pushes,
                interface_target.obj_pushes,
            ),
            (1, 0, 0, 0)
        );
        assert_eq!(
            (
                interface_drag.int_pops,
                interface_drag.obj_pops,
                interface_drag.int_pushes,
                interface_drag.obj_pushes,
            ),
            (1, 0, 0, 0)
        );
    }

    #[test]
    fn command_signatures_supply_entities_login_and_misc_stack_effects() {
        let highlight_toggle =
            opcode_stack_effect("unknown_command_68").expect("entity signature effect");
        let highlight_state =
            opcode_stack_effect("unknown_command_69").expect("entity signature effect");
        let openstore = opcode_stack_effect("openstore").expect("login signature effect");
        let platformtype = opcode_stack_effect("platformtype").expect("misc signature effect");
        let social_network_947 =
            opcode_stack_effect_for_build("lobby_enterlobby_social_network", 947)
                .expect("build-specific social network login effect");

        assert_eq!(
            (highlight_toggle.int_pops, highlight_toggle.int_pushes),
            (1, 0)
        );
        assert_eq!(
            (highlight_state.int_pops, highlight_state.int_pushes),
            (0, 1)
        );
        assert_eq!(
            (openstore.int_pops, openstore.obj_pops, openstore.int_pushes),
            (1, 1, 0)
        );
        assert_eq!((platformtype.int_pops, platformtype.int_pushes), (0, 1));
        assert_eq!(
            (social_network_947.int_pops, social_network_947.obj_pops),
            (4, 0)
        );
    }

    #[test]
    fn manual_opcode_effects_cover_misc_residual_signatures() {
        let clear_hooks =
            opcode_stack_effect("cc_clearscripthooks").expect("cc clear hooks effect");
        let object_desc = opcode_stack_effect("oc_desc").expect("object desc effect");
        let inventory_stockbase =
            opcode_stack_effect("inv_stockbase").expect("inventory stockbase effect");
        let interface_vflip = opcode_stack_effect("if_setvflip").expect("if vflip effect");
        let steam_achievement =
            opcode_stack_effect("steam_setachievement").expect("steam achievement effect");
        let npc_custom_head =
            opcode_stack_effect("if_npc_setcustomheadmodel").expect("if npc custom head effect");
        let camera_axis =
            opcode_stack_effect("cam2_setpositionacceleration_axis").expect("camera axis effect");
        let custom_body_retex =
            opcode_stack_effect("cc_setcustombodyretex").expect("cc custom retex effect");
        let store_lookup = opcode_stack_effect("store_lookup").expect("store lookup effect");
        let field6563 = opcode_stack_effect("field6563").expect("field6563 effect");
        let struct_param = opcode_stack_effect("struct_param").expect("struct param effect");
        let camera_depth = opcode_stack_effect("cam2_setdepthplanes").expect("camera depth effect");
        let char_index = opcode_stack_effect("if_getcharindexatpos").expect("if char index effect");
        let bounding_box =
            opcode_stack_effect("get_entity_bounding_box").expect("entity bounding box effect");
        let worldmap_coord = opcode_stack_effect("worldmap_3dview_getcoordfine")
            .expect("world map coord fine effect");
        let resume_long =
            opcode_stack_effect("resume_countdialog_long").expect("resume long effect");
        let marketing_event = opcode_stack_effect("marketing_sendanalyticsevent")
            .expect("marketing analytics effect");
        let highlight_scale =
            opcode_stack_effect("highlight_set_category_scale").expect("highlight scale effect");
        let highlight_silhouette = opcode_stack_effect("highlight_set_localplayer_silhouette_mode")
            .expect("highlight silhouette effect");
        let unknown_65 = opcode_stack_effect("unknown_command_65").expect("unknown 65 effect");
        let legacy_detail = opcode_stack_effect("field5283").expect("legacy detail effect");
        let field6317 = opcode_stack_effect("field6317").expect("field6317 effect");

        assert_eq!((clear_hooks.int_pops, clear_hooks.int_pushes), (0, 0));
        assert_eq!((field6563.int_pops, field6563.int_pushes), (0, 1));
        assert_eq!((struct_param.int_pops, struct_param.int_pushes), (2, 1));
        assert_eq!((camera_depth.int_pops, camera_depth.int_pushes), (2, 0));
        assert_eq!((char_index.int_pops, char_index.int_pushes), (3, 1));
        assert_eq!((bounding_box.int_pops, bounding_box.int_pushes), (0, 5));
        assert_eq!((worldmap_coord.int_pops, worldmap_coord.int_pushes), (3, 1));
        assert_eq!((resume_long.obj_pops, resume_long.int_pushes), (1, 0));
        assert_eq!(
            (marketing_event.obj_pops, marketing_event.int_pushes),
            (1, 0)
        );
        assert_eq!(
            (highlight_scale.int_pops, highlight_scale.int_pushes),
            (2, 0)
        );
        assert_eq!(
            (
                highlight_silhouette.int_pops,
                highlight_silhouette.int_pushes,
            ),
            (1, 0)
        );
        assert_eq!((unknown_65.int_pops, unknown_65.int_pushes), (1, 0));
        assert_eq!((legacy_detail.int_pops, legacy_detail.int_pushes), (1, 1));
        assert_eq!((field6317.int_pops, field6317.int_pushes), (1, 0));
        assert_eq!(
            (inventory_stockbase.int_pops, inventory_stockbase.int_pushes),
            (2, 1)
        );
        assert_eq!(
            (interface_vflip.int_pops, interface_vflip.int_pushes),
            (2, 0)
        );
        assert_eq!(
            (
                object_desc.int_pops,
                object_desc.obj_pops,
                object_desc.int_pushes,
                object_desc.obj_pushes,
            ),
            (1, 0, 0, 1)
        );
        assert_eq!(
            (
                steam_achievement.int_pops,
                steam_achievement.obj_pops,
                steam_achievement.int_pushes,
                steam_achievement.obj_pushes,
            ),
            (2, 1, 1, 0)
        );
        assert_eq!(
            (npc_custom_head.int_pops, npc_custom_head.int_pushes),
            (3, 0)
        );
        assert_eq!((camera_axis.int_pops, camera_axis.int_pushes), (4, 0));
        assert_eq!(
            (custom_body_retex.int_pops, custom_body_retex.int_pushes),
            (4, 0)
        );
        assert_eq!(
            (
                store_lookup.int_pops,
                store_lookup.obj_pops,
                store_lookup.int_pushes,
                store_lookup.long_pushes,
            ),
            (1, 1, 10, 3)
        );
    }

    #[test]
    fn interface_option_commands_use_payload_stack_effects() {
        let effect = opcode_stack_effect("if_setop")
            .expect("if_setop should use full option payload stack effect");
        let key_effect = opcode_stack_effect("if_setopkey")
            .expect("if_setopkey should use full key payload stack effect");

        assert_eq!(
            (
                effect.int_pops,
                effect.obj_pops,
                effect.long_pops,
                effect.int_pushes,
                effect.obj_pushes,
                effect.long_pushes,
            ),
            (2, 1, 0, 0, 0, 0)
        );
        assert_eq!(
            (
                key_effect.int_pops,
                key_effect.obj_pops,
                key_effect.long_pops,
                key_effect.int_pushes,
                key_effect.obj_pushes,
                key_effect.long_pushes,
            ),
            (4, 0, 0, 0, 0, 0)
        );
    }

    #[test]
    fn if_setop_recovers_option_text_and_component() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(1),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::String(String::new()),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(83_230_736),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "if_setop".to_string(),
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
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "UI.Setop")
                    && call.arguments.len() == 3
                    && matches!(&call.arguments[1], Expression::StringLiteral(value) if value.value.is_empty())
        ));
    }

    #[test]
    fn interface_createchild_recovers_full_payload() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(10),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(1),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(2),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(0),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "if_createchild".to_string(),
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
            &recovered[4],
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if matches!(&*call.callee, Expression::Identifier(id) if id.name == "UI.Createchild")
                    && call.arguments.len() == 4
        ));
        assert!(matches!(&recovered[5], Some(RecoveredStmt::Return(None))));
    }

    #[test]
    fn interface_single_payload_methods_recover_component_argument() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(1),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(0),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "if_setnpchead".to_string(),
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
            &recovered[2],
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if matches!(&*call.callee, Expression::Identifier(id) if id.name == "UI.Setnpchead")
                    && call.arguments.len() == 2
        ));
        assert!(matches!(&recovered[3], Some(RecoveredStmt::Return(None))));
    }

    #[test]
    fn return_recovers_all_stack_values() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(1),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(2),
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
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "stack")
                    && call.arguments.len() == 2
                    && matches!(&call.arguments[0], Expression::Identifier(identifier) if identifier.name == "local_int_1")
                    && matches!(&call.arguments[1], Expression::Identifier(identifier) if identifier.name == "local_int_2")
        ));
    }

    #[test]
    fn command_signatures_supply_maths_long_stack_effects() {
        let effect = opcode_stack_effect("int_to_long")
            .expect("maths signature should provide int_to_long stack effect");

        assert_eq!(
            (
                effect.int_pops,
                effect.obj_pops,
                effect.long_pops,
                effect.int_pushes,
                effect.obj_pushes,
                effect.long_pushes,
            ),
            (1, 0, 0, 0, 0, 1)
        );
    }

    #[test]
    fn command_signatures_supply_camera_stack_effects() {
        let effect = opcode_stack_effect("cam2_setfieldofviewscreen")
            .expect("camera signature should provide screen FOV stack effect");

        assert_eq!(
            (
                effect.int_pops,
                effect.obj_pops,
                effect.long_pops,
                effect.int_pushes,
                effect.obj_pushes,
                effect.long_pushes,
            ),
            (3, 0, 0, 0, 0, 0)
        );
    }

    #[test]
    fn tostring_uses_build_specific_extra_argument_after_919() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(0),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(10),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "tostring".to_string(),
                operand: OperandNode::Int(0),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "return".to_string(),
                operand: OperandNode::Int(0),
            },
        ];

        let recovered = ExprRecovery::new_for_build(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
            947,
        )
        .recover();

        assert!(matches!(
            &recovered[3],
            Some(RecoveredStmt::Return(Some(Expression::Call(call))))
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "tostring")
                    && call.arguments.len() == 2
        ));
    }

    #[test]
    fn opcode_stack_effect_reports_build_specific_tostring_arity() {
        let legacy = opcode_stack_effect_for_build("tostring", 910)
            .expect("legacy tostring effect should exist");
        let modern = opcode_stack_effect_for_build("tostring", 947)
            .expect("modern tostring effect should exist");

        assert_eq!(legacy.int_pops, 1);
        assert_eq!(modern.int_pops, 2);
    }

    #[test]
    fn opcode_stack_effect_reports_build_specific_clan_find_affined_arity() {
        let legacy = opcode_stack_effect_for_build("activeclansettings_find_affined", 910)
            .expect("legacy clan find effect should exist");
        let modern = opcode_stack_effect_for_build("activeclansettings_find_affined", 947)
            .expect("modern clan find effect should exist");

        assert_eq!((legacy.int_pops, legacy.int_pushes), (0, 1));
        assert_eq!((modern.int_pops, modern.int_pushes), (1, 1));
    }

    #[test]
    fn opcode_stack_effect_reports_build_specific_camera_entity_arity() {
        let legacy = opcode_stack_effect_for_build("cam2_setpositionentity_npc", 910)
            .expect("legacy camera entity effect should exist");
        let modern = opcode_stack_effect_for_build("cam2_setpositionentity_npc", 947)
            .expect("modern camera entity effect should exist");

        assert_eq!(legacy.int_pops, 7);
        assert_eq!(modern.int_pops, 8);
    }

    #[test]
    fn modern_clan_find_affined_recovers_mode_argument() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(0),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "activeclansettings_find_affined".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "return".to_string(),
                operand: OperandNode::Byte(0),
            },
        ];

        let recovered = ExprRecovery::new_for_build(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
            947,
        )
        .recover();

        assert!(matches!(
            &recovered[2],
            Some(RecoveredStmt::Return(Some(Expression::Call(call))))
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "activeclansettingsfindaffined")
                    && call.arguments.len() == 1
        ));
    }

    #[test]
    fn avatar_base_setters_recover_payload_arguments() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(2),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(3),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "basecolour".to_string(),
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
            &recovered[2],
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "basecolour")
                    && call.arguments.len() == 2
        ));
        assert!(matches!(&recovered[3], Some(RecoveredStmt::Return(None))));
    }

    #[test]
    fn generic_opcode_calls_recover_nonzero_mode_operand() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(93),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(0),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "inv_total".to_string(),
                operand: OperandNode::Byte(1),
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
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "invtotalWithMode")
                    && call.arguments.len() == 3
                    && matches!(&call.arguments[2], Expression::NumberLiteral(number) if number.value == 1)
        ));
    }

    #[test]
    fn plain_strings_with_callback_like_text_stay_strings() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::String("oc_debugname".to_string()),
            },
            InstructionNode {
                index: 1,
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
            Some(RecoveredStmt::Return(Some(Expression::StringLiteral(value))))
                if value.value == "oc_debugname"
        ));
    }

    #[test]
    fn discarded_value_call_recovers_expression_statement() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(5),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::String("entry".to_string()),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(-1),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "cc_list_addentry".to_string(),
                operand: OperandNode::Int(0),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "pop_int_discard".to_string(),
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
            &recovered[4],
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "UI.ListAddentry")
                    && call.arguments.len() == 3
        ));
    }

    #[test]
    fn delayed_stack_value_before_void_command_recovers_push_sequence() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(0),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "autosetup_setultra".to_string(),
                operand: OperandNode::Int(0),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "pop_int_discard".to_string(),
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
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "stackpush_then")
                    && call.arguments.len() == 2
        ));
        assert!(matches!(
            &recovered[2],
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "popintdiscard")
        ));
    }

    #[test]
    fn delayed_stack_value_before_value_command_recovers_push_sequence() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(0),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "unknown_command_20".to_string(),
                operand: OperandNode::Int(0),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(1),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "branch_equals".to_string(),
                operand: OperandNode::Branch(4),
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
            Some(RecoveredStmt::BranchBinary { left: Expression::Call(call), .. })
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "stackpush_then")
                    && call.arguments.len() == 2
        ));
    }

    #[test]
    fn empty_discard_recovers_typed_discard_call() {
        let instructions = vec![InstructionNode {
            index: 0,
            opcode: 0,
            command: "pop_int_discard".to_string(),
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
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "popintdiscard")
                    && call.arguments.is_empty()
        ));
    }

    #[test]
    fn scriptqueue_add_recovers_delay_callback_and_component() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(50),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(19746),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(-2_147_483_645),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(-2_147_483_643),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::String("ii".to_string()),
            },
            InstructionNode {
                index: 5,
                opcode: 0,
                command: "cc_scriptqueue_add".to_string(),
                operand: OperandNode::Int(0),
            },
            InstructionNode {
                index: 6,
                opcode: 0,
                command: "pop_long_discard".to_string(),
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

        assert!(recovered[5].is_none());
        assert!(matches!(
            &recovered[6],
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "UI.ScriptqueueAdd")
                    && call.arguments.len() == 2
                    && matches!(&call.arguments[1], Expression::CallbackLiteral(callback) if callback.script == "script19746" && callback.arguments.len() == 2)
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
    fn cc_create_preserves_nonzero_operand_mode() {
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
                operand: OperandNode::Int(5),
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
                command: "cc_create".to_string(),
                operand: OperandNode::Byte(1),
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
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "UI.create")
                    && call.arguments.len() == 4
                    && matches!(&call.arguments[3], Expression::NumberLiteral(mode) if mode.value == 1)
        ));
    }

    #[test]
    fn ui_call_recovery_preserves_nonzero_operand_mode_suffix() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::String("text".to_string()),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "cc_settext".to_string(),
                operand: OperandNode::Byte(1),
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
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "UI.setTextWithMode")
                    && call.arguments.len() == 2
                    && matches!(&call.arguments[1], Expression::NumberLiteral(mode) if mode.value == 1)
        ));
    }

    #[test]
    fn ui_hook_preserves_pending_stack_values_before_statement() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(99),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(5327),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::String(String::new()),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(0),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "if_setonclick".to_string(),
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
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "stackpush_then")
                    && matches!(call.arguments.first(), Some(Expression::NumberLiteral(NumberLiteral { value: 99 })))
                    && matches!(call.arguments.last(), Some(Expression::Call(inner))
                        if matches!(&*inner.callee, Expression::Identifier(identifier) if identifier.name == "UI.Setonclick"))
        ));
    }

    #[test]
    fn ui_hook_callback_count_recovers_from_enum_named_literal() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(123),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(10),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(11),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(2),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::String("Y".to_string()),
            },
            InstructionNode {
                index: 5,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(0),
            },
            InstructionNode {
                index: 6,
                opcode: 0,
                command: "if_setonvarcstrtransmit".to_string(),
                operand: OperandNode::Byte(0),
            },
        ];
        let enum_value_names = HashMap::from([(2, "Enum_1.TWO".to_string())]);

        let recovered = ExprRecovery::new(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &enum_value_names,
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        assert!(matches!(
            &recovered[6],
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "UI.Setonvarcstrtransmit")
                    && matches!(call.arguments.first(), Some(Expression::CallbackLiteral(callback))
                        if callback.script == "script123"
                            && callback.watchers == ["varcstr_10", "varcstr_11"])
        ));
    }

    #[test]
    fn varbit_recovery_preserves_transmog_suffix() {
        let instructions = vec![InstructionNode {
            index: 0,
            opcode: 0,
            command: "push_varbit".to_string(),
            operand: OperandNode::VarBitRef(VarBitRefNode {
                id: VarBitId(45522),
                name: Some("varplayerbit_45522".to_string()),
                is_transmog: true,
            }),
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

        assert!(recovered[0].is_none());
        assert!(matches!(
            ExprRecovery::new(
                &[
                    instructions[0].clone(),
                    InstructionNode {
                        index: 1,
                        opcode: 0,
                        command: "return".to_string(),
                        operand: OperandNode::Int(0),
                    },
                ],
                &HashMap::new(),
                &HashMap::<u32, String>::new(),
                &HashMap::<i32, String>::new(),
                &ScriptCatalog::default(),
                &HashMap::new(),
            )
            .recover()
            .get(1),
            Some(Some(RecoveredStmt::Return(Some(Expression::Identifier(identifier)))))
                if identifier.name == "varplayerbit_45522_transmog"
        ));
    }

    #[test]
    fn var_recovery_preserves_transmog_suffix() {
        let instructions = [InstructionNode {
            index: 0,
            opcode: 0,
            command: "push_var".to_string(),
            operand: OperandNode::VarRef(VarRefNode {
                domain: VarDomain::Player,
                id: VarId(12655),
                name: Some("varplayer_12655".to_string()),
                is_transmog: true,
            }),
        }];

        assert!(matches!(
            ExprRecovery::new(
                &[
                    instructions[0].clone(),
                    InstructionNode {
                        index: 1,
                        opcode: 0,
                        command: "return".to_string(),
                        operand: OperandNode::Int(0),
                    },
                ],
                &HashMap::new(),
                &HashMap::<u32, String>::new(),
                &HashMap::<i32, String>::new(),
                &ScriptCatalog::default(),
                &HashMap::new(),
            )
            .recover()
            .get(1),
            Some(Some(RecoveredStmt::Return(Some(Expression::Identifier(identifier)))))
                if identifier.name == "varplayer_12655_transmog"
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
    fn find_recovery_preserves_nonzero_operand_mode() {
        let cc_instructions = vec![
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
                operand: OperandNode::Byte(1),
            },
        ];
        let if_instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(100),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "if_find".to_string(),
                operand: OperandNode::Byte(1),
            },
        ];

        let recovered_cc = ExprRecovery::new(
            &cc_instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();
        let recovered_if = ExprRecovery::new(
            &if_instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        assert!(recovered_cc[2].is_none());
        assert!(recovered_if[1].is_none());
        // Both are value-producing calls left on the recovered stack and will
        // be consumed by a following branch/store/return in real scripts; use
        // return to observe the expression without exposing internals.
        let mut cc_with_return = cc_instructions;
        cc_with_return.push(InstructionNode {
            index: 3,
            opcode: 0,
            command: "return".to_string(),
            operand: OperandNode::Byte(0),
        });
        let mut if_with_return = if_instructions;
        if_with_return.push(InstructionNode {
            index: 2,
            opcode: 0,
            command: "return".to_string(),
            operand: OperandNode::Byte(0),
        });
        let recovered_cc = ExprRecovery::new(
            &cc_with_return,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();
        let recovered_if = ExprRecovery::new(
            &if_with_return,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
        )
        .recover();

        assert!(matches!(
            &recovered_cc[3],
            Some(RecoveredStmt::Return(Some(Expression::Call(call))))
                if call.arguments.len() == 3
        ));
        assert!(matches!(
            &recovered_if[2],
            Some(RecoveredStmt::Return(Some(Expression::Call(call))))
                if matches!(&*call.callee, Expression::Identifier(identifier) if identifier.name == "UI.findInterface")
                    && call.arguments.len() == 2
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
                if matches!(&*call.callee, Expression::Identifier(id) if id.name == "stack")
                    && call.arguments.len() == 2
                    && matches!(&call.arguments[0], Expression::Call(inner)
                        if matches!(&*inner.callee, Expression::Identifier(id) if id.name == "dblistall"))
                    && matches!(&call.arguments[1], Expression::Call(inner)
                        if matches!(&*inner.callee, Expression::Identifier(id) if id.name == "dbgetrowtable"))
        ));
        assert!(recovered[1].is_none());
    }

    #[test]
    fn recovered_return_type_uses_string_return_values() {
        let recovered = vec![Some(RecoveredStmt::Return(Some(
            Expression::StringLiteral(StringLiteral {
                value: "ok".to_string(),
            }),
        )))];

        assert_eq!(detect_return_type_from_recovered(&recovered), "string");
    }

    #[test]
    fn recovered_return_type_ignores_void_calls_inside_stack_return() {
        let script_id = ScriptId(1);
        let mut catalog = ScriptCatalog::default();
        let signature = ScriptSignature {
            arg_count_int: 1,
            arg_count_obj: 0,
            arg_count_long: 0,
            return_count_int: 0,
            return_count_obj: 0,
            return_count_long: 0,
            return_type: "void".to_string(),
        };
        catalog.insert(ScriptMetadata {
            packed_id: script_id,
            group_id: ScriptGroupId(1),
            file_id: 0,
            kind: ScriptKind::Unknown,
            raw_name: None,
            short_name: "void_helper".to_string(),
            export_name: "void_helper".to_string(),
            module_name: "void_helper".to_string(),
            signature: signature.clone(),
        });
        let signatures = HashMap::from([(script_id, signature)]);
        let recovered = vec![Some(RecoveredStmt::Return(Some(Expression::Call(
            CallExpr {
                callee: Box::new(Expression::Identifier(Identifier {
                    name: "stack".to_string(),
                })),
                arguments: vec![
                    Expression::Call(CallExpr {
                        callee: Box::new(Expression::Identifier(Identifier {
                            name: "void_helper".to_string(),
                        })),
                        arguments: Vec::new(),
                    }),
                    Expression::StringLiteral(StringLiteral {
                        value: "ok".to_string(),
                    }),
                ],
            },
        ))))];

        assert_eq!(
            detect_return_type_from_recovered_with_signatures(&recovered, &catalog, &signatures),
            "string"
        );
    }

    #[test]
    fn db_find_uses_build_specific_basevartype_argument_after_919() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(503_808),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(0),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "push_constant_string".to_string(),
                operand: OperandNode::Int(0),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "db_find".to_string(),
                operand: OperandNode::Byte(0),
            },
        ];

        let recovered = ExprRecovery::new_for_build(
            &instructions,
            &HashMap::new(),
            &HashMap::<u32, String>::new(),
            &HashMap::<i32, String>::new(),
            &ScriptCatalog::default(),
            &HashMap::new(),
            947,
        )
        .recover();

        assert!(matches!(
            &recovered[3],
            Some(RecoveredStmt::Expression(Expression::Call(call)))
                if matches!(&*call.callee, Expression::Identifier(id) if id.name == "dbfind")
                    && call.arguments.len() == 3
        ));
    }

    #[test]
    fn void_zero_arg_commands_recover_as_statements() {
        let instructions = vec![InstructionNode {
            index: 0,
            opcode: 0,
            command: "notifications_init".to_string(),
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
                if matches!(&*call.callee, Expression::Identifier(id) if id.name == "notificationsinit")
                    && call.arguments.is_empty()
        ));
    }

    #[test]
    fn cam2_mode_setters_recover_stack_argument() {
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
                command: "cam2_setlookatmode".to_string(),
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
                if matches!(&*call.callee, Expression::Identifier(id) if id.name == "cam2setlookatmode")
                    && call.arguments.len() == 1
        ));
    }

    #[test]
    fn db_filter_value_recovers_four_arguments_and_result() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(1_470_560),
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
                operand: OperandNode::Int(3),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "push_constant_int".to_string(),
                operand: OperandNode::Int(0),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "db_filter_value".to_string(),
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
                if matches!(&*call.callee, Expression::Identifier(id) if id.name == "dbfiltervalue")
                    && call.arguments.len() == 4
        ));
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
    fn minimenu_unknown_helpers_preserve_multivalue_stack_order() {
        let instructions = vec![
            InstructionNode {
                index: 0,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(1),
            },
            InstructionNode {
                index: 1,
                opcode: 0,
                command: "push_int_local".to_string(),
                operand: OperandNode::Local(2),
            },
            InstructionNode {
                index: 2,
                opcode: 0,
                command: "unknown_command_29".to_string(),
                operand: OperandNode::Byte(0),
            },
            InstructionNode {
                index: 3,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(6),
            },
            InstructionNode {
                index: 4,
                opcode: 0,
                command: "pop_string_local".to_string(),
                operand: OperandNode::Local(0),
            },
            InstructionNode {
                index: 5,
                opcode: 0,
                command: "pop_int_local".to_string(),
                operand: OperandNode::Local(5),
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

        for (slot, index) in [(&recovered[3], 2), (&recovered[4], 1), (&recovered[5], 0)] {
            assert!(matches!(
                slot,
                Some(RecoveredStmt::Assignment { value: Expression::ArrayAccess(access), .. })
                    if matches!(&*access.array, Expression::Call(call)
                        if matches!(&*call.callee, Expression::Identifier(id) if id.name == "unknowncommand29"))
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
                if matches!(&*call.callee, Expression::Identifier(id) if id.name == "stack")
                    && call.arguments.len() == 3
                    && matches!(&call.arguments[0], Expression::Call(inner)
                        if matches!(&*inner.callee, Expression::Identifier(id) if id.name == "UI.HasSub"))
                    && matches!(&call.arguments[1], Expression::Call(inner)
                        if matches!(&*inner.callee, Expression::Identifier(id) if id.name == "UI.HasSubmodal"))
                    && matches!(&call.arguments[2], Expression::Call(inner)
                        if matches!(&*inner.callee, Expression::Identifier(id) if id.name == "UI.GetGamescreen"))
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
                                && matches!(call.arguments.as_slice(), [Expression::Call(arg)]
                                    if matches!(&*arg.callee, Expression::Identifier(id) if id.name == "intconst")
                                        && matches!(arg.arguments.as_slice(), [Expression::NumberLiteral(crate::transpile::ast::NumberLiteral { value: 302 })])))
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
