//! Semantic CS2 type lattice — a Rust port of zwyz/rs3-cache `Type.java` +
//! `Lattice.java`.
//!
//! Whole-program type inference (G3, see `plans/tooling/cs2-runescript-decompiler.md`)
//! refines the three raw VM base types (int / long / object) into the ~190 semantic
//! CS2 types (`component`, `loc`, `enum`, `dbcolumn`, …). This module is the lattice
//! those inferred types live in: a partial order where *lower = more specific*
//! (`loc < unknown_int < unknown`), with `meet` choosing the most-specific common
//! refinement and `conflict` as the bottom ("no type possible").
//!
//! Presentation-only: it drives typed/named local rendering. It never affects
//! encoded bytes — the byte-fidelity gate is downstream and unaware of it.
//!
//! ## Port notes / scope
//! - Builds 910 + 948 are ≥ 751, so only the numeric `byID` type encoding is ported
//!   (zwyz's `byChar`, for builds < 751, is intentionally omitted).
//! - The exotic `unknown1..50` / `varp` / `transmit_list` placeholder types are
//!   `byChar`-only (absent from `byID` and from our command-signature corpus); they
//!   are omitted and [`TypeLattice::by_name`] falls back to `unknown` for any
//!   unrecognised signature type rather than panicking.
//! - The generic `Lattice` transitive-closure construction is order-independent for
//!   the resulting relation, so the (slightly) different vertex set vs zwyz does not
//!   change `meet`/`test` on the shared types; only rare `meet` tie-breaks among
//!   mutually-incomparable common lower bounds could differ, which is immaterial for
//!   display typing.

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::OnceLock;

/// The underlying stack representation of a type (what the VM actually stores).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BaseVarType {
    Integer,
    Long,
    String,
    CoordFine,
}

/// A handle into the global [`TypeLattice`] (an interned index). Cheap to copy;
/// only meaningful against [`lattice()`] (there is exactly one global instance).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Type(pub u32);

impl Type {
    /// Display name (the RuneScript type keyword, e.g. `component`).
    pub fn name(self) -> &'static str {
        lattice().name(self)
    }

    /// The stack base type, if this is a "real" (non-set, non-fake) type.
    pub fn base(self) -> Option<BaseVarType> {
        lattice().data(self).base
    }

    /// The array type whose elements are `self` (e.g. `component` → `componentarray`).
    /// `None` for array types themselves (no arrays-of-arrays).
    pub fn array(self) -> Option<Self> {
        lattice().data(self).array
    }

    /// The element type if `self` is an array type, else `None`.
    pub fn element(self) -> Option<Self> {
        lattice().data(self).element
    }

    /// `true` if `self <= other` in the lattice (self is the same or more specific).
    pub fn is_subtype_of(self, other: Self) -> bool {
        lattice().test(self, other)
    }

    /// The most-specific common refinement of the two types (greatest lower bound).
    /// Returns the `conflict` bottom type when the only common lower bound is bottom.
    pub fn meet(self, other: Self) -> Self {
        lattice().meet(self, other)
    }
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, Clone)]
struct TypeData {
    name: &'static str,
    base: Option<BaseVarType>,
    alias: Option<Type>,
    array: Option<Type>,
    element: Option<Type>,
}

/// Well-known type handles needed by the lattice construction, the propagator, and
/// rendering. Captured at build time (some, like `int` vs `int_int`, share a name and
/// cannot be looked up by name unambiguously).
#[derive(Debug, Clone, Copy)]
pub struct WellKnown {
    pub int: Type,
    pub int_int: Type,
    pub boolean: Type,
    pub obj: Type,
    pub namedobj: Type,
    pub string: Type,
    pub coordfine: Type,
    pub component: Type,
    pub char: Type,
    pub int_key: Type,
    pub unknown: Type,
    pub unknown_int: Type,
    pub unknown_int_notboolean: Type,
    pub unknown_int_notint: Type,
    pub unknown_int_notint_notboolean: Type,
    pub unknown_long: Type,
    pub unknown_object: Type,
    pub unknown_array: Type,
    pub conflict: Type,
    pub var_player: Type,
    pub var_player_bit: Type,
}

#[derive(Default)]
pub struct TypeLattice {
    types: Vec<TypeData>,
    by_name: HashMap<&'static str, Type>,
    by_id: HashMap<i32, Type>,
    upper: Vec<HashSet<Type>>,
    lower: Vec<HashSet<Type>>,
    well_known: Option<WellKnown>,
}

impl TypeLattice {
    fn data(&self, t: Type) -> &TypeData {
        &self.types[t.0 as usize]
    }

    pub fn name(&self, t: Type) -> &'static str {
        self.types[t.0 as usize].name
    }

    /// Look up a type by its RuneScript keyword (as it appears in `data/commands/*.txt`).
    /// Unrecognised names resolve to `unknown` (robust against signature drift).
    pub fn by_name(&self, name: &str) -> Type {
        // `unwrap_or_else` keeps the fallback lazy: during `build()` the well-known
        // table isn't set yet, but every build-time lookup is for an already-registered
        // name, so the fallback never fires there.
        self.by_name
            .get(name)
            .copied()
            .unwrap_or_else(|| self.wk().unknown)
    }

    /// Look up a type by its numeric id (the build ≥ 751 encoding used by `enum`
    /// input/output operands and `define_array` element types).
    pub fn by_id(&self, id: i32) -> Option<Type> {
        self.by_id.get(&id).copied()
    }

    pub fn wk(&self) -> &WellKnown {
        self.well_known.as_ref().expect("lattice well-known not set")
    }

    /// `a <= b`: is `a` the same as, or more specific than, `b`?
    pub fn test(&self, a: Type, b: Type) -> bool {
        self.upper[a.0 as usize].contains(&b)
    }

    /// Greatest lower bound (most-specific common refinement). Mirrors zwyz's
    /// `Lattice.meet`: intersect the common subtypes, then walk up to the maximum.
    pub fn meet(&self, a: Type, b: Type) -> Type {
        if a == b {
            return a;
        }
        if self.test(a, b) {
            return a;
        }
        if self.test(b, a) {
            return b;
        }
        // Common lower bounds, in registration order (deterministic tie-break).
        let mut max: Option<Type> = None;
        for i in 0..self.types.len() as u32 {
            let x = Type(i);
            if self.test(x, a) && self.test(x, b) {
                max = Some(match max {
                    None => x,
                    Some(m) if self.test(m, x) => x,
                    Some(m) => m,
                });
            }
        }
        // `conflict` is below everything, so a common lower bound always exists.
        max.expect("meet has no common lower bound (conflict missing?)")
    }

    fn array_of(&self, t: Type) -> Type {
        self.data(t)
            .array
            .expect("array_of called on an array type")
    }

    // ── construction ────────────────────────────────────────────────────────

    fn register(
        &mut self,
        name: &'static str,
        base: Option<BaseVarType>,
        alias: Option<Type>,
    ) -> Type {
        let t = Type(self.types.len() as u32);
        self.types.push(TypeData {
            name,
            base,
            alias,
            array: None,
            element: None,
        });
        self.upper.push(HashSet::from([t]));
        self.lower.push(HashSet::from([t]));
        // zwyz's BY_NAME.put keeps the *last* registration for a duplicate name
        // (e.g. "int" → INT_INT). Replicate by always inserting.
        self.by_name.insert(name, t);
        t
    }

    /// Create the array type for every non-array type (eager, as in zwyz's ctor).
    fn register_array(&mut self, element: Type) -> Type {
        // Leak the "<name>array" string to obtain a 'static name, matching the
        // interned-name model (only ~200, done once at startup).
        let name: &'static str = Box::leak(format!("{}array", self.name(element)).into_boxed_str());
        let t = Type(self.types.len() as u32);
        self.types.push(TypeData {
            name,
            base: Some(BaseVarType::Integer),
            alias: None,
            array: None,
            element: Some(element),
        });
        self.upper.push(HashSet::from([t]));
        self.lower.push(HashSet::from([t]));
        self.by_name.insert(name, t);
        self.types[element.0 as usize].array = Some(t);
        t
    }

    fn add(&mut self, a: Type, b: Type) {
        if self.test(a, b) {
            return;
        }
        let lowers_a: Vec<Type> = self.lower[a.0 as usize].iter().copied().collect();
        let uppers_b: Vec<Type> = self.upper[b.0 as usize].iter().copied().collect();
        for &x in &lowers_a {
            for &y in &uppers_b {
                self.upper[x.0 as usize].insert(y);
                self.lower[y.0 as usize].insert(x);
            }
        }
    }
}

/// The single global lattice instance.
pub fn lattice() -> &'static TypeLattice {
    static LATTICE: OnceLock<TypeLattice> = OnceLock::new();
    LATTICE.get_or_init(build)
}

fn build() -> TypeLattice {
    let mut l = TypeLattice::default();

    // ── named "real" types (lines 14..276 of Type.java; byChar-only exotics
    //    unknownN/varp/transmit_list omitted — see module docs) ──
    // Single-letter base markers mirror zwyz's compact type table.
    const I: Option<BaseVarType> = Some(BaseVarType::Integer);
    const L: Option<BaseVarType> = Some(BaseVarType::Long);
    const S: Option<BaseVarType> = Some(BaseVarType::String);
    const C: Option<BaseVarType> = Some(BaseVarType::CoordFine);

    // (name, base) in declaration order. Order is immaterial to the relation but
    // gives meet a deterministic tie-break consistent with zwyz's insertion order.
    let named: &[(&'static str, Option<BaseVarType>)] = &[
        ("int", I),
        ("boolean", I),
        ("hash32", I),
        ("quest", I),
        ("questhelp", I),
        ("cursor", I),
        ("seq", I),
        ("colour", I),
        ("locshape", I),
        ("component", I),
        ("idkit", I),
        ("midi", I),
        ("npc_mode", I),
        ("synth", I),
        ("ai_queue", I),
        ("area", I),
        ("stat", I),
        ("npc_stat", I),
        ("writeinv", I),
        ("mesh", I),
        ("wma", I),
        ("coord", I),
        ("graphic", I),
        ("chatphrase", I),
        ("fontmetrics", I),
        ("enum", I),
        ("hunt", I),
        ("jingle", I),
        ("chatcat", I),
        ("loc", I),
        ("model", I),
        ("npc", I),
        ("obj", I),
        ("namedobj", I),
        ("player_uid", I),
        ("region_uid", L),
        ("string", S),
        ("spotanim", I),
        ("npc_uid", I),
        ("inv", I),
        ("texture", I),
        ("category", I),
        ("char", I),
        ("laser", I),
        ("bas", I),
        ("controller", I),
        ("collision_geometry", I),
        ("physics_model", I),
        ("physics_control_modifier", I),
        ("clanhash", L),
        ("coordfine", C),
        ("cutscene", I),
        ("itemcode", I),
        ("pvpkills", I),
        ("msi", I),
        ("clanforumqfc", L),
        ("vorbis", I),
        ("verifyobj", I),
        ("mapelement", I),
        ("categorytype", I),
        ("socialnetwork", I),
        ("hitmark", I),
        ("package", I),
        ("pef", I),
        ("controller_uid", I),
        ("pem", I),
        ("plog", I),
        ("unsigned_int", I),
        ("skybox", I),
        ("skydecor", I),
        ("hash64", L),
        ("inputtype", I),
        ("struct", I),
        ("dbrow", I),
        ("storablelabel", I),
        ("storableproc", I),
        ("gamelogevent", I),
        ("animationclip", I),
        ("skeleton", I),
        ("region_visibility", I),
        ("fmodhandle", I),
        ("region_allowlogin", I),
        ("region_info", I),
        ("region_info_failure", I),
        ("server_account_creation_step", I),
        ("client_account_creation_step", I),
        ("lobby_account_creation_step", I),
        ("gwc_platform", I),
        ("currency", I),
        ("keyboard_key", I),
        ("mouseevent", I),
        ("headbar", I),
        ("bugtemplate", I),
        ("billingauthflag", I),
        ("accountfeatureflag", I),
        ("interface", I),
        ("toplevelinterface", I),
        ("overlayinterface", I),
        ("clientinterface", I),
        ("movespeed", I),
        ("material", I),
        ("seqgroup", I),
        ("TEMPHISCORE", I),
        ("temphiscorelengthtype", I),
        ("temphiscoretype", I),
        ("temphiscorecontributeresult", I),
        ("audiogroup", I),
        ("audiobuss", I),
        ("long", L),
        ("crm_channel", I),
        ("http_image", I),
        ("popupdisplaybehaviour", I),
        ("poll", I),
        ("mtxn_package", L),
        ("mtxn_price_point", L),
        ("pointlight", I),
        ("player_group", L),
        ("player_group_status", I),
        ("player_group_invite_result", I),
        ("player_group_modify_result", I),
        ("player_group_join_or_create_result", I),
        ("player_group_affinity_modify_result", I),
        ("player_group_delta_type", I),
        ("client_type", I),
        ("telemetry_interval", I),
        ("worldarea", I),
        ("dbtable", I),
        ("achievement", I),
        ("stylesheet", I),
        ("ui_anim_curve", I),
        ("ui_anim", I),
        ("anim_state_machine", I),
        ("cutscene2d", I),
        // group 2
        ("label", I),
        ("queue", I),
        ("timer", I),
        ("weakqueue", I),
        ("softtimer", I),
        ("objvar", I),
        ("walktrigger", I),
        ("var_int", I),
        ("var_long", I),
        ("var_string", I),
        // unknown id (config-ish)
        ("vfx", I),
        ("mesanim", I),
        ("underlay", I),
        ("overlay", I),
        ("light", I),
        ("water", I),
        ("billboard", I),
        // special
        ("type", I),
        ("basevartype", I),
        ("param", I),
        ("clientscript", I),
        ("twitch_event", I),
        ("minimenu_event", I),
        ("dbcolumn", I),
        ("dbfilter", I),
        ("var_player", I),
        ("var_player_bit", I),
        ("var_npc", I),
        ("var_npc_bit", I),
        ("var_client", I),
        ("var_client_bit", I),
        ("var_client_string", I),
        ("var_world", I),
        ("var_world_bit", I),
        ("var_world_string", I),
        ("var_region", I),
        ("var_region_bit", I),
        ("var_object", I),
        ("var_object_bit", I),
        ("var_clan", I),
        ("var_clan_bit", I),
        ("var_clan_setting", I),
        ("var_clan_setting_bit", I),
        ("var_controller", I),
        ("var_controller_bit", I),
        ("var_player_group", I),
        ("var_player_group_bit", I),
        ("var_global", I),
        ("var_global_bit", I),
        ("sound", I),
    ];
    for &(name, base) in named {
        l.register(name, base, None);
    }
    // Capture the *base* int before INT_INT shadows the "int" name.
    let int = l.by_name("int");

    // ── "split int" alias subtypes (alias = base int) ──
    let alias_int: &[&'static str] = &[
        "int", // INT_INT — re-registers "int", shadowing the base int in by_name
        "intbool",
        "chatfilter",
        "chattype",
        "platformtype",
        "iftype",
        "key",
        "setposh",
        "setposv",
        "setsize",
        "settextalignh",
        "settextalignv",
        "windowmode",
        "rgb",
        "clientoption",
        "filterop",
    ];
    for &name in alias_int {
        l.register(name, Some(BaseVarType::Integer), Some(int));
    }
    let int_int = l.by_name("int"); // now resolves to INT_INT

    // ── type sets + fake unpacker types ──
    let unknown = l.register("unknown", None, None);
    let unknown_int = l.register("unknown_int", None, None);
    let unknown_int_notboolean = l.register("unknown_int_notboolean", None, None);
    let unknown_int_notint = l.register("unknown_int_notint", None, None);
    let unknown_int_notint_notboolean = l.register("unknown_int_notint_notboolean", None, None);
    let unknown_long = l.register("unknown_long", None, None);
    let unknown_object = l.register("unknown_object", None, None);
    let conflict = l.register("conflict", None, None);
    let _hook = l.register("hook", None, None);
    let _condition = l.register("condition", None, None);

    // ── arrays for every non-array type registered so far ──
    let base_count = l.types.len();
    for i in 0..base_count as u32 {
        l.register_array(Type(i));
    }

    // ── lattice edges (Type.java static block) ──
    let boolean = l.by_name("boolean");
    let namedobj = l.by_name("namedobj");
    let obj = l.by_name("obj");
    let string = l.by_name("string");
    let coordfine = l.by_name("coordfine");

    // first loop: place every type under its umbrella set
    for i in 0..l.types.len() as u32 {
        let t = Type(i);
        let d = l.data(t).clone();
        match d.base {
            Some(BaseVarType::Integer) => {
                if d.alias == Some(int) {
                    l.add(t, int);
                    l.add(int_int, t);
                } else if t == int {
                    l.add(t, unknown_int_notboolean);
                } else if t == boolean {
                    l.add(t, unknown_int_notint);
                } else {
                    l.add(t, unknown_int_notint_notboolean);
                }
            }
            Some(BaseVarType::Long) => l.add(t, unknown_long),
            _ => l.add(t, unknown),
        }
    }

    l.add(unknown_int, unknown);
    l.add(unknown_object, unknown);
    l.add(unknown_long, unknown);
    l.add(l.array_of(unknown), unknown_int);
    l.add(unknown_int_notint, unknown_int);
    l.add(unknown_int_notboolean, unknown_int);
    l.add(unknown_int_notint_notboolean, unknown_int_notint);
    l.add(unknown_int_notint_notboolean, unknown_int_notboolean);
    l.add(string, unknown_object);
    l.add(coordfine, unknown_object);
    l.add(namedobj, obj);

    // mirror the lattice for arrays: x <= y  ⇒  xarray <= yarray
    for i in 0..base_count as u32 {
        let x = Type(i);
        let uppers: Vec<Type> = l.upper[x.0 as usize].iter().copied().collect();
        let xa = l.array_of(x);
        for y in uppers {
            // y may be a set type without an array (all registered base types have
            // one; sets registered above base_count also have one) — guard anyway.
            if let Some(ya) = l.data(y).array {
                l.add(xa, ya);
            }
        }
    }

    // bottom type: conflict <= everything
    for i in 0..l.types.len() as u32 {
        l.add(conflict, Type(i));
    }

    // ── by_id (build ≥ 751 numeric encoding) ──
    let id_names: &[(i32, &'static str)] = &[
        (0, "int"), // special-cased below to the *base* int
        (1, "boolean"),
        (2, "hash32"),
        (3, "quest"),
        (4, "questhelp"),
        (5, "cursor"),
        (6, "seq"),
        (7, "colour"),
        (8, "locshape"),
        (9, "component"),
        (10, "idkit"),
        (11, "midi"),
        (12, "npc_mode"),
        (13, "namedobj"),
        (14, "synth"),
        (15, "ai_queue"),
        (16, "area"),
        (17, "stat"),
        (18, "npc_stat"),
        (19, "writeinv"),
        (20, "mesh"),
        (21, "wma"),
        (22, "coord"),
        (23, "graphic"),
        (24, "chatphrase"),
        (25, "fontmetrics"),
        (26, "enum"),
        (27, "hunt"),
        (28, "jingle"),
        (29, "chatcat"),
        (30, "loc"),
        (31, "model"),
        (32, "npc"),
        (33, "obj"),
        (34, "player_uid"),
        (35, "region_uid"),
        (36, "string"),
        (37, "spotanim"),
        (38, "npc_uid"),
        (39, "inv"),
        (40, "texture"),
        (41, "category"),
        (42, "char"),
        (43, "laser"),
        (44, "bas"),
        (45, "controller"),
        (46, "collision_geometry"),
        (47, "physics_model"),
        (48, "physics_control_modifier"),
        (49, "clanhash"),
        (50, "coordfine"),
        (51, "cutscene"),
        (53, "itemcode"),
        (54, "pvpkills"),
        (55, "msi"),
        (56, "clanforumqfc"),
        (57, "vorbis"),
        (58, "verifyobj"),
        (59, "mapelement"),
        (60, "categorytype"),
        (61, "socialnetwork"),
        (62, "hitmark"),
        (63, "package"),
        (64, "pef"),
        (65, "controller_uid"),
        (66, "pem"),
        (67, "plog"),
        (68, "unsigned_int"),
        (69, "skybox"),
        (70, "skydecor"),
        (71, "hash64"),
        (72, "inputtype"),
        (73, "struct"),
        (74, "dbrow"),
        (75, "storablelabel"),
        (76, "storableproc"),
        (77, "gamelogevent"),
        (78, "animationclip"),
        (79, "skeleton"),
        (80, "region_visibility"),
        (81, "fmodhandle"),
        (83, "region_allowlogin"),
        (84, "region_info"),
        (85, "region_info_failure"),
        (86, "server_account_creation_step"),
        (87, "client_account_creation_step"),
        (88, "lobby_account_creation_step"),
        (89, "gwc_platform"),
        (90, "currency"),
        (91, "keyboard_key"),
        (92, "mouseevent"),
        (93, "headbar"),
        (94, "bugtemplate"),
        (95, "billingauthflag"),
        (96, "accountfeatureflag"),
        (97, "interface"),
        (98, "toplevelinterface"),
        (99, "overlayinterface"),
        (100, "clientinterface"),
        (101, "movespeed"),
        (102, "material"),
        (103, "seqgroup"),
        (104, "TEMPHISCORE"),
        (105, "temphiscorelengthtype"),
        (106, "temphiscoretype"),
        (107, "temphiscorecontributeresult"),
        (108, "audiogroup"),
        (109, "audiobuss"),
        (110, "long"),
        (111, "crm_channel"),
        (112, "http_image"),
        (113, "popupdisplaybehaviour"),
        (114, "poll"),
        (115, "mtxn_package"),
        (116, "mtxn_price_point"),
        (117, "pointlight"),
        (118, "player_group"),
        (119, "player_group_status"),
        (120, "player_group_invite_result"),
        (121, "player_group_modify_result"),
        (122, "player_group_join_or_create_result"),
        (123, "player_group_affinity_modify_result"),
        (124, "player_group_delta_type"),
        (125, "client_type"),
        (126, "telemetry_interval"),
        (127, "worldarea"),
        (129, "dbtable"),
        (131, "achievement"),
        (133, "stylesheet"),
        (135, "ui_anim_curve"),
        (136, "ui_anim"),
        (137, "anim_state_machine"),
        (138, "cutscene2d"),
        // 200/201 are array ids
        (202, "label"),
        (203, "queue"),
        (204, "timer"),
        (205, "weakqueue"),
        (206, "softtimer"),
        (207, "objvar"),
        (208, "walktrigger"),
        (209, "var_int"),
        (210, "var_long"),
        (211, "var_string"),
    ];
    for &(id, name) in id_names {
        let t = if id == 0 { int } else { l.by_name(name) };
        l.by_id.insert(id, t);
    }
    // array type ids
    l.by_id.insert(200, l.array_of(l.by_name("component")));
    l.by_id.insert(201, l.array_of(int));

    let wk = WellKnown {
        int,
        int_int,
        boolean,
        obj,
        namedobj,
        string,
        coordfine,
        component: l.by_name("component"),
        char: l.by_name("char"),
        int_key: l.by_name("key"),
        unknown,
        unknown_int,
        unknown_int_notboolean,
        unknown_int_notint,
        unknown_int_notint_notboolean,
        unknown_long,
        unknown_object,
        unknown_array: l.array_of(unknown),
        conflict,
        var_player: l.by_name("var_player"),
        var_player_bit: l.by_name("var_player_bit"),
    };
    l.well_known = Some(wk);
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wk() -> WellKnown {
        *lattice().wk()
    }

    #[test]
    fn names_and_ids_resolve() {
        let l = lattice();
        assert_eq!(l.by_id(9).expect("id 9").name(), "component");
        assert_eq!(l.by_id(30).expect("id 30").name(), "loc");
        assert_eq!(l.by_id(26).expect("id 26").name(), "enum");
        // id 0 is the *base* int, not the int_int alias
        assert_eq!(l.by_id(0).expect("id 0"), wk().int);
        assert_eq!(l.by_name("dbcolumn").name(), "dbcolumn");
        // by_name("int") is the alias int_int (shadowing), per zwyz
        assert_eq!(l.by_name("int"), wk().int_int);
        // unrecognised → unknown fallback (no panic)
        assert_eq!(l.by_name("definitely_not_a_type"), wk().unknown);
    }

    #[test]
    fn reflexive_and_bottom() {
        let l = lattice();
        let comp = wk().component;
        assert!(l.test(comp, comp));
        // conflict <= everything
        for i in 0..200u32 {
            assert!(
                l.test(wk().conflict, Type(i)),
                "conflict must subtype {}",
                Type(i).name()
            );
        }
    }

    #[test]
    fn specific_types_subtype_unknown_int() {
        let l = lattice();
        for n in ["component", "loc", "enum", "npc", "stat", "dbcolumn"] {
            let t = l.by_name(n);
            assert!(t.is_subtype_of(wk().unknown_int), "{n} <= unknown_int");
            assert!(t.is_subtype_of(wk().unknown), "{n} <= unknown");
            assert!(
                !wk().unknown_int.is_subtype_of(t),
                "unknown_int must NOT subtype {n}"
            );
        }
    }

    #[test]
    fn meet_specific_with_set_keeps_specific() {
        // loc meet unknown_int = loc (the set is an ancestor)
        let loc = lattice().by_name("loc");
        assert_eq!(loc.meet(wk().unknown_int), loc);
        assert_eq!(wk().unknown_int.meet(loc), loc);
        assert_eq!(loc.meet(wk().unknown), loc);
    }

    #[test]
    fn meet_conflicting_aliases_is_int_int() {
        // two distinct int aliases collapse to int_int (erases the bad alias)
        let intbool = lattice().by_name("intbool");
        let key = lattice().by_name("key");
        assert_eq!(intbool.meet(key), wk().int_int);
    }

    #[test]
    fn meet_unrelated_is_conflict() {
        // component and loc are mutually incomparable, distinct int subtypes
        let comp = lattice().by_name("component");
        let loc = lattice().by_name("loc");
        assert_eq!(comp.meet(loc), wk().conflict);
    }

    #[test]
    fn namedobj_subtypes_obj() {
        assert!(wk().namedobj.is_subtype_of(wk().obj));
        assert_eq!(wk().namedobj.meet(wk().obj), wk().namedobj);
    }

    #[test]
    fn arrays_mirror_and_element() {
        let comp = wk().component;
        let comp_arr = comp.array().expect("component has an array type");
        assert_eq!(comp_arr.name(), "componentarray");
        assert_eq!(comp_arr.element(), Some(comp));
        assert!(comp_arr.is_subtype_of(wk().unknown_int));
        // arrays-of-arrays don't exist
        assert_eq!(comp_arr.array(), None);
    }

    #[test]
    fn base_types_classified() {
        assert_eq!(wk().string.base(), Some(BaseVarType::String));
        assert_eq!(wk().int.base(), Some(BaseVarType::Integer));
        assert_eq!(lattice().by_name("long").base(), Some(BaseVarType::Long));
        assert_eq!(wk().coordfine.base(), Some(BaseVarType::CoordFine));
        // string/coordfine live under unknown_object
        assert!(wk().string.is_subtype_of(wk().unknown_object));
        assert!(wk().coordfine.is_subtype_of(wk().unknown_object));
    }
}
