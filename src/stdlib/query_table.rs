//! The standard library's query table, one line per query.
//!
//! A line is `CONSTANT = "suffix", return type, reads`, optionally followed by `=>` and modifiers:
//! `args(min, max)` / `min_args(min)` (default: any number), `until(v)` (resolves up to version `v`
//! only), `split(v)` (two version ranges, up to `v` and after it), `tags()` / `world_gen()` (query
//! set; default `default`), `client()` / `server()` (side), `default(..)` (no-subject default),
//! `not_on_server()` (not resolved by a server catalogue) and `since(major, minor, patch)` (first
//! release that has it).
//!
//! Argument counts and sides feed only the lints; compiling never rejects an expression on them.

use crate::catalog::{
    Arity, DefaultReturn, QueryCatalog, QueryDecl, QuerySetMask, QueryShape, QuerySide, Reads,
    ReturnType, Side, VersionRange, VersionRanges,
};
use crate::version::{MolangVersion, semver::Version};

/// One row of the table while its modifiers apply; [`Row::shape`] makes it a [`QueryShape`].
#[derive(Copy, Clone, Debug)]
struct Row {
    args: Arity,
    returns: ReturnType,
    reads: Reads,
    side: QuerySide,
    default_return: DefaultReturn,
    since: Option<(u64, u64, u64)>,
    versions: Versions,
    sets: QuerySetMask,
}

/// The Molang versions a row resolves at.
#[derive(Copy, Clone, Debug)]
enum Versions {
    All,
    /// Up to this version.
    Until(i16),
    /// Up to this version, and after it in a second range.
    Split(i16),
}

impl Row {
    const fn new(returns: ReturnType, reads: Reads) -> Self {
        Self {
            args: QueryShape::DEFAULT.args,
            returns,
            reads,
            side: QueryShape::DEFAULT.side,
            default_return: QueryShape::DEFAULT.default_return,
            since: None,
            versions: Versions::All,
            sets: QuerySetMask::DEFAULT,
        }
    }

    const fn args(self, min: u8, max: u8) -> Self {
        Self {
            args: Arity::between(min, max),
            ..self
        }
    }

    const fn min_args(self, min: u8) -> Self {
        Self {
            args: Arity::at_least(min),
            ..self
        }
    }

    const fn until(self, last: i16) -> Self {
        Self {
            versions: Versions::Until(last),
            ..self
        }
    }

    const fn split(self, last: i16) -> Self {
        Self {
            versions: Versions::Split(last),
            ..self
        }
    }

    const fn tags(self) -> Self {
        Self {
            sets: QuerySetMask::TAGS,
            ..self
        }
    }

    const fn world_gen(self) -> Self {
        Self {
            sets: QuerySetMask::WORLD_GEN,
            ..self
        }
    }

    const fn client(self) -> Self {
        Self {
            side: QuerySide::CLIENT,
            ..self
        }
    }

    const fn server(self) -> Self {
        Self {
            side: QuerySide::Server,
            ..self
        }
    }

    const fn default(self, default_return: DefaultReturn) -> Self {
        Self {
            default_return,
            ..self
        }
    }

    const fn not_on_server(self) -> Self {
        let side = match self.side {
            QuerySide::Both { .. } => QuerySide::Both {
                on_dedicated_server: false,
            },
            QuerySide::Client { .. } => QuerySide::Client {
                on_dedicated_server: false,
            },
            QuerySide::Server => panic!("a server query resolves on a dedicated server"),
        };
        Self { side, ..self }
    }

    const fn since(self, major: u64, minor: u64, patch: u64) -> Self {
        Self {
            since: Some((major, minor, patch)),
            ..self
        }
    }

    /// The shape. A bound that is not a version fails the build of the table.
    const fn shape(self) -> QueryShape {
        const fn range(first: i16, last: i16, sets: QuerySetMask) -> VersionRange {
            let (Some(first), Some(last)) = (
                MolangVersion::from_i16(first),
                MolangVersion::from_i16(last),
            ) else {
                panic!("not a Molang version")
            };
            let Some(range) = VersionRange::new(first, last, sets) else {
                panic!("a reversed version range")
            };
            range
        }
        let latest = MolangVersion::LATEST.as_i16();
        let ranges = match self.versions {
            Versions::All => VersionRanges::single(range(0, latest, self.sets)),
            Versions::Until(last) => VersionRanges::single(range(0, last, self.sets)),
            Versions::Split(last) => {
                let first = VersionRanges::single(range(0, last, self.sets));
                let Some(two) = first.followed_by(range(last + 1, latest, self.sets)) else {
                    panic!("overlapping version ranges")
                };
                two
            }
        };
        QueryShape {
            args: self.args,
            returns: self.returns,
            ranges,
            experiments: QueryShape::DEFAULT.experiments,
            reads: self.reads,
            side: self.side,
            default_return: self.default_return,
            first_release: match self.since {
                Some((major, minor, patch)) => Some(Version::new(major, minor, patch)),
                None => None,
            },
        }
    }
}

/// The standard library's catalogue of `side`, without the queries first present after `release`.
pub(super) fn catalog(side: Side, release: Option<&Version>) -> QueryCatalog {
    let rows = ROWS.iter().filter(|(_, shape)| {
        shape
            .first_release
            .as_ref()
            .is_none_or(|since| release.is_none_or(|release| since <= release))
    });
    // `every_row_is_a_valid_declaration` pins both.
    let decls = rows.map(|(name, shape)| {
        QueryDecl::new(name, shape.clone()).expect("every row is a valid declaration")
    });
    QueryCatalog::new(side, decls).expect("the rows declare distinct names")
}

const F: ReturnType = ReturnType::FLOAT;
const B: ReturnType = ReturnType::BOOL;
const HASH: ReturnType = ReturnType::STRING;
const MATRIX: ReturnType = ReturnType::MATRIX;
const STRUCT: ReturnType = ReturnType::STRUCT;
const ACTOR_REF: ReturnType = ReturnType::ACTOR;
const ACTOR_ARRAY: ReturnType = ReturnType::ACTOR_ARRAY;

const NONE: Reads = Reads::empty();
const ACTOR: Reads = Reads::ACTOR;
const ITEM: Reads = Reads::ITEM;
const BLOCK: Reads = Reads::BLOCK;
const LEVEL: Reads = Reads::LEVEL;
const WORLD_GEN: Reads = Reads::WORLD_GEN;
const CAMERA: Reads = Reads::CAMERA;
const RENDER: Reads = Reads::RENDER;
const CLIENT_STATE: Reads = Reads::CLIENT_STATE;
const VARIABLES: Reads = Reads::VARIABLES;

use DefaultReturn::{EmptyActorArray, EmptyString, Float1, FloatNeg1, StructRgba0};

macro_rules! queries {
    ($($konst:ident = $suffix:literal, $returns:ident, $reads:expr $(=> $($modifier:ident($($arg:expr),*)).+)?;)*) => {
        /// The full name of every standard query: `query::BLOCK_STATE` is `"query.block_state"`.
        pub mod query {
            $(
                #[doc = concat!("`query.", $suffix, "`.")]
                pub const $konst: &str = concat!("query.", $suffix);
            )*
        }

        /// Sorted by name.
        static ROWS: &[(&str, QueryShape)] = &[$((query::$konst, Row::new($returns, $reads)$($(.$modifier($($arg),*))+)?.shape())),*];
    };
}

queries! {
    ABOVE_TOP_SOLID = "above_top_solid", F, WORLD_GEN => world_gen();
    ACTOR_COUNT = "actor_count", F, CLIENT_STATE => client();
    ALL = "all", F, NONE => min_args(3);
    ALL_ANIMATIONS_FINISHED = "all_animations_finished", B, RENDER => client();
    ALL_TAGS = "all_tags", F, ITEM.union(BLOCK) => tags();
    ANGER_LEVEL = "anger_level", F, ACTOR => args(0, 0).server();
    ANIM_TIME = "anim_time", F, RENDER => client();
    ANY = "any", F, NONE => min_args(3);
    ANY_ANIMATION_FINISHED = "any_animation_finished", B, RENDER => client();
    ANY_TAG = "any_tag", F, ITEM.union(BLOCK) => tags();
    APPROX_EQ = "approx_eq", B, NONE => min_args(2);
    ARMOR_COLOR_SLOT = "armor_color_slot", F, ACTOR => args(2, 2).default(Float1);
    ARMOR_DAMAGE_SLOT = "armor_damage_slot", F, ACTOR => args(1, 1);
    ARMOR_MATERIAL_SLOT = "armor_material_slot", F, ACTOR => args(1, 1);
    ARMOR_TEXTURE_SLOT = "armor_texture_slot", F, ACTOR => args(1, 1);
    AVERAGE_FRAME_TIME = "average_frame_time", F, CLIENT_STATE => args(0, 1).client();
    BASE_SWING_DURATION = "base_swing_duration", F, ACTOR;
    BLOCK_FACE = "block_face", F, VARIABLES;
    BLOCK_HAS_ALL_TAGS = "block_has_all_tags", B, BLOCK => min_args(4);
    BLOCK_HAS_ANY_TAG = "block_has_any_tag", B, BLOCK => min_args(4);
    BLOCK_NEIGHBOR_HAS_ALL_TAGS = "block_neighbor_has_all_tags", B, BLOCK => min_args(1);
    BLOCK_NEIGHBOR_HAS_ANY_TAG = "block_neighbor_has_any_tag", B, BLOCK => min_args(1);
    BLOCK_PROPERTY = "block_property", B, BLOCK => args(1, 1).until(9);
    BLOCK_STATE = "block_state", B, BLOCK => args(1, 1);
    BLOCKING = "blocking", B, ACTOR;
    BODY_X_ROTATION = "body_x_rotation", F, ACTOR.union(RENDER);
    BODY_Y_ROTATION = "body_y_rotation", F, ACTOR.union(RENDER);
    BONE_AABB = "bone_aabb", MATRIX, CLIENT_STATE => args(1, 1).client();
    BONE_ORIENTATION_MATRIX = "bone_orientation_matrix", MATRIX, RENDER => args(1, 2).client();
    BONE_ORIENTATION_TRS = "bone_orientation_trs", MATRIX, RENDER => args(1, 2).client();
    BONE_ORIGIN = "bone_origin", MATRIX, CLIENT_STATE => args(1, 1).client();
    BONE_ROTATION = "bone_rotation", MATRIX, CLIENT_STATE => args(1, 1).client();
    CAMERA_DISTANCE_RANGE_LERP = "camera_distance_range_lerp", F, CAMERA => args(2, 2).client();
    CAMERA_ROTATION = "camera_rotation", F, CLIENT_STATE => args(1, 1).client();
    CAN_CLIMB = "can_climb", B, ACTOR;
    CAN_DAMAGE_NEARBY_MOBS = "can_damage_nearby_mobs", B, ACTOR;
    CAN_DASH = "can_dash", B, ACTOR;
    CAN_FLY = "can_fly", B, ACTOR;
    CAN_POWER_JUMP = "can_power_jump", B, ACTOR;
    CAN_SWIM = "can_swim", B, ACTOR;
    CAN_WALK = "can_walk", B, ACTOR;
    CAPE_FLAP_AMOUNT = "cape_flap_amount", F, ACTOR => split(7);
    CARDINAL_BLOCK_FACE_PLACED_ON = "cardinal_block_face_placed_on", F, VARIABLES;
    CARDINAL_FACING = "cardinal_facing", F, ACTOR;
    CARDINAL_FACING_2D = "cardinal_facing_2d", F, ACTOR;
    CARDINAL_PLAYER_FACING = "cardinal_player_facing", F, ACTOR;
    CLIENT_MAX_RENDER_DISTANCE = "client_max_render_distance", F, CLIENT_STATE => args(0, 0).client();
    CLIENT_MEMORY_TIER = "client_memory_tier", F, CLIENT_STATE => args(0, 0).client();
    COMBINE_ENTITIES = "combine_entities", ACTOR_ARRAY, ACTOR => default(EmptyActorArray);
    COOLDOWN_TIME = "cooldown_time", F, ACTOR => args(1, 2);
    COOLDOWN_TIME_REMAINING = "cooldown_time_remaining", F, ACTOR => args(0, 2);
    COUNT = "count", F, NONE;
    CURRENT_SQUISH_VALUE = "current_squish_value", F, ACTOR;
    DASH_COOLDOWN_PROGRESS = "dash_cooldown_progress", F, ACTOR => until(10).client();
    DAY = "day", F, ACTOR => args(0, 0);
    DEATH_TICKS = "death_ticks", F, ACTOR;
    DEBUG_OUTPUT = "debug_output", F, NONE => args(1, 1);
    DELTA_TIME = "delta_time", F, RENDER => client();
    DISTANCE_FROM_CAMERA = "distance_from_camera", F, CAMERA => client();
    EFFECT_EMITTER_COUNT = "effect_emitter_count", F, CLIENT_STATE => client();
    EFFECT_PARTICLE_COUNT = "effect_particle_count", F, CLIENT_STATE => client();
    ENTITY_BIOME_HAS_ALL_TAGS = "entity_biome_has_all_tags", B, ACTOR.union(BLOCK) => min_args(1).client();
    ENTITY_BIOME_HAS_ANY_IDENTIFIER = "entity_biome_has_any_identifier", B, ACTOR.union(BLOCK) => min_args(1).client();
    ENTITY_BIOME_HAS_ANY_TAGS = "entity_biome_has_any_tags", B, ACTOR.union(BLOCK) => min_args(1).client();
    EQUIPMENT_COUNT = "equipment_count", F, ACTOR => args(0, 0);
    EQUIPPED_ITEM_ALL_TAGS = "equipped_item_all_tags", B, ACTOR => args(0, 1);
    EQUIPPED_ITEM_ANY_TAG = "equipped_item_any_tag", B, ACTOR => args(0, 1);
    EQUIPPED_ITEM_IS_ATTACHABLE = "equipped_item_is_attachable", ACTOR_REF, CLIENT_STATE => args(1, 1).client();
    EYE_TARGET_X_ROTATION = "eye_target_x_rotation", F, ACTOR;
    EYE_TARGET_Y_ROTATION = "eye_target_y_rotation", F, ACTOR;
    FACING_TARGET_TO_RANGE_ATTACK = "facing_target_to_range_attack", B, ACTOR;
    FRAME_ALPHA = "frame_alpha", F, RENDER => client();
    FUSE_TIME = "fuse_time", F, ACTOR => since(1, 26, 30);
    GET_ACTOR_INFO_ID = "get_actor_info_id", F, LEVEL => args(1, 1);
    GET_ANIMATION_FRAME = "get_animation_frame", F, ACTOR => args(0, 0);
    GET_DEFAULT_BONE_PIVOT = "get_default_bone_pivot", F, ACTOR.union(RENDER) => client();
    GET_EQUIPPED_ITEM_NAME = "get_equipped_item_name", HASH, ACTOR.union(RENDER) => args(0, 2).default(EmptyString);
    GET_LEVEL_SEED_BASED_FRACTION = "get_level_seed_based_fraction", F, ACTOR.union(BLOCK).union(LEVEL) => args(0, 0);
    GET_LOCATOR_OFFSET = "get_locator_offset", F, RENDER => client();
    GET_NAME = "get_name", HASH, ACTOR => args(0, 1);
    GET_PACK_SETTING = "get_pack_setting", F, ACTOR.union(LEVEL) => args(1, 1).client();
    GET_ROOT_LOCATOR_OFFSET = "get_root_locator_offset", F, RENDER => args(2, 2).client();
    GRAPHICS_MODE_IS_ANY = "graphics_mode_is_any", F, CLIENT_STATE => min_args(1).client();
    GROUND_SPEED = "ground_speed", F, ACTOR;
    HAD_COMPONENT_GROUP = "had_component_group", B, ACTOR => args(1, 1);
    HAS_ALL_BIOME_TAGS = "has_all_biome_tags", B, WORLD_GEN => world_gen().since(1, 26, 50);
    HAS_ANY_BIOME_TAGS = "has_any_biome_tags", B, WORLD_GEN => world_gen().since(1, 26, 50);
    HAS_ANY_FAMILY = "has_any_family", B, ACTOR => min_args(1);
    HAS_ANY_LEASHED_ENTITY_OF_TYPE = "has_any_leashed_entity_of_type", B, ACTOR;
    HAS_ARMOR_SLOT = "has_armor_slot", F, ACTOR => args(1, 1);
    HAS_BIOME_TAG = "has_biome_tag", B, WORLD_GEN => world_gen();
    HAS_BLOCK_PROPERTY = "has_block_property", B, BLOCK => args(1, 1).until(9);
    HAS_BLOCK_STATE = "has_block_state", B, BLOCK => args(1, 1);
    HAS_CAPE = "has_cape", B, ACTOR;
    HAS_COLLISION = "has_collision", B, ACTOR;
    HAS_DASH_COOLDOWN = "has_dash_cooldown", B, ACTOR;
    HAS_GRAVITY = "has_gravity", B, ACTOR;
    HAS_HEAD_GEAR = "has_head_gear", B, ACTOR;
    HAS_OWNER = "has_owner", B, ACTOR;
    HAS_PLAYER_RIDER = "has_player_rider", B, ACTOR => args(0, 0);
    HAS_PROPERTY = "has_property", F, ACTOR => args(1, 1);
    HAS_RIDER = "has_rider", B, ACTOR;
    HAS_TARGET = "has_target", B, ACTOR;
    HEAD_IS_IN_WATER = "head_is_in_water", B, ACTOR => since(1, 26, 50);
    HEAD_ROLL_ANGLE = "head_roll_angle", F, ACTOR;
    HEAD_X_ROTATION = "head_x_rotation", F, ACTOR.union(RENDER) => min_args(1);
    HEAD_Y_ROTATION = "head_y_rotation", F, ACTOR.union(RENDER) => min_args(1);
    HEALTH = "health", F, ACTOR => args(0, 0);
    HEARTBEAT_INTERVAL = "heartbeat_interval", F, ACTOR => args(0, 0);
    HEARTBEAT_PHASE = "heartbeat_phase", F, ACTOR => args(0, 0).client();
    HEIGHTMAP = "heightmap", F, WORLD_GEN => world_gen();
    HURT_DIRECTION = "hurt_direction", F, ACTOR => args(0, 0);
    HURT_TIME = "hurt_time", F, ACTOR => args(0, 0);
    IN_RANGE = "in_range", F, NONE => args(3, 3);
    INVULNERABLE_TICKS = "invulnerable_ticks", F, ACTOR;
    IS_ADMIRING = "is_admiring", B, ACTOR;
    IS_ALIVE = "is_alive", B, ACTOR;
    IS_ANGRY = "is_angry", B, ACTOR;
    IS_ATTACHED = "is_attached", B, ACTOR => args(0, 0).client();
    IS_ATTACHED_TO_ENTITY = "is_attached_to_entity", B, ACTOR;
    IS_AVOIDING_BLOCK = "is_avoiding_block", B, ACTOR;
    IS_AVOIDING_MOBS = "is_avoiding_mobs", B, ACTOR;
    IS_BABY = "is_baby", B, ACTOR;
    IS_BREATHING = "is_breathing", B, ACTOR;
    IS_BRIBED = "is_bribed", B, ACTOR;
    IS_CARRYING_BLOCK = "is_carrying_block", B, ACTOR => split(12);
    IS_CASTING = "is_casting", B, ACTOR;
    IS_CELEBRATING = "is_celebrating", B, ACTOR;
    IS_CELEBRATING_SPECIAL = "is_celebrating_special", B, ACTOR;
    IS_CHARGED = "is_charged", B, ACTOR;
    IS_CHARGING = "is_charging", B, ACTOR;
    IS_CHESTED = "is_chested", B, ACTOR;
    IS_COOLDOWN_CATEGORY = "is_cooldown_category", B, ACTOR => args(1, 2);
    IS_CRAWLING = "is_crawling", B, ACTOR;
    IS_CRITICAL = "is_critical", B, ACTOR;
    IS_CROAKING = "is_croaking", B, ACTOR;
    IS_DANCING = "is_dancing", B, ACTOR;
    IS_DELAYED_ATTACKING = "is_delayed_attacking", B, ACTOR;
    IS_DIGGING = "is_digging", B, ACTOR;
    IS_EATING = "is_eating", B, ACTOR;
    IS_EATING_MOB = "is_eating_mob", B, ACTOR;
    IS_ELDER = "is_elder", B, ACTOR;
    IS_EMERGING = "is_emerging", B, ACTOR;
    IS_EMOTING = "is_emoting", B, ACTOR;
    IS_ENCHANTED = "is_enchanted", B, ACTOR;
    IS_FEELING_HAPPY = "is_feeling_happy", B, ACTOR => args(0, 0).until(10);
    IS_FIRE_IMMUNE = "is_fire_immune", B, ACTOR;
    IS_FIRST_PERSON = "is_first_person", B, RENDER => client();
    IS_GHOST = "is_ghost", B, ACTOR;
    IS_GLIDING = "is_gliding", B, ACTOR;
    IS_GRAZING = "is_grazing", B, ACTOR;
    IS_IDLING = "is_idling", B, ACTOR;
    IS_IGNITED = "is_ignited", B, ACTOR;
    IS_ILLAGER_CAPTAIN = "is_illager_captain", B, ACTOR;
    IS_IN_CONTACT_WITH_WATER = "is_in_contact_with_water", B, ACTOR;
    IS_IN_LAVA = "is_in_lava", B, ACTOR;
    IS_IN_LOVE = "is_in_love", B, ACTOR;
    IS_IN_UI = "is_in_ui", B, ACTOR;
    IS_IN_WATER = "is_in_water", B, ACTOR;
    IS_IN_WATER_OR_RAIN = "is_in_water_or_rain", B, ACTOR;
    IS_INTERESTED = "is_interested", B, ACTOR;
    IS_INVISIBLE = "is_invisible", B, ACTOR;
    IS_ITEM_EQUIPPED = "is_item_equipped", B, ACTOR => args(0, 1);
    IS_ITEM_NAME_ANY = "is_item_name_any", B, ACTOR => min_args(2);
    IS_JUMP_GOAL_JUMPING = "is_jump_goal_jumping", B, ACTOR;
    IS_JUMPING = "is_jumping", B, ACTOR;
    IS_LAYING_DOWN = "is_laying_down", B, ACTOR;
    IS_LAYING_EGG = "is_laying_egg", B, ACTOR;
    IS_LEASHED = "is_leashed", B, ACTOR;
    IS_LEVITATING = "is_levitating", B, ACTOR;
    IS_LINGERING = "is_lingering", B, ACTOR;
    IS_LOCAL_PLAYER = "is_local_player", B, ACTOR => client();
    IS_MOVING = "is_moving", B, ACTOR;
    IS_NAME_ANY = "is_name_any", B, ACTOR => min_args(1);
    IS_ON_FIRE = "is_on_fire", B, ACTOR;
    IS_ON_GROUND = "is_on_ground", B, ACTOR;
    IS_ON_SCREEN = "is_on_screen", B, ACTOR => not_on_server();
    IS_ONFIRE = "is_onfire", B, ACTOR;
    IS_ORPHANED = "is_orphaned", B, ACTOR;
    IS_OWNER_IDENTIFIER_ANY = "is_owner_identifier_any", B, ACTOR => min_args(1);
    IS_PACK_SETTING_ENABLED = "is_pack_setting_enabled", B, ACTOR.union(LEVEL).union(RENDER) => args(1, 1).client();
    IS_PACK_SETTING_SELECTED = "is_pack_setting_selected", B, ACTOR.union(LEVEL) => args(2, 2).client();
    IS_PERSONA_OR_PREMIUM_SKIN = "is_persona_or_premium_skin", B, ACTOR => args(0, 0);
    IS_PLAYING_DEAD = "is_playing_dead", B, ACTOR;
    IS_POWERED = "is_powered", B, ACTOR;
    IS_PREGNANT = "is_pregnant", B, ACTOR;
    IS_RAM_ATTACKING = "is_ram_attacking", B, ACTOR;
    IS_RESTING = "is_resting", B, ACTOR;
    IS_RIDING = "is_riding", B, ACTOR;
    IS_RIDING_ANY_ENTITY_OF_TYPE = "is_riding_any_entity_of_type", B, ACTOR;
    IS_RISING = "is_rising", B, ACTOR => args(0, 0).until(10);
    IS_ROARING = "is_roaring", B, ACTOR;
    IS_ROLLING = "is_rolling", B, ACTOR;
    IS_SADDLED = "is_saddled", B, ACTOR;
    IS_SCARED = "is_scared", B, ACTOR;
    IS_SCENTING = "is_scenting", B, ACTOR => args(0, 0).until(10);
    IS_SEARCHING = "is_searching", B, ACTOR => args(0, 0);
    IS_SELECTED_ITEM = "is_selected_item", B, ACTOR;
    IS_SHAKING = "is_shaking", B, ACTOR;
    IS_SHAKING_WETNESS = "is_shaking_wetness", F, ACTOR;
    IS_SHEARED = "is_sheared", B, ACTOR;
    IS_SHIELD_POWERED = "is_shield_powered", B, ACTOR;
    IS_SILENT = "is_silent", B, ACTOR;
    IS_SITTING = "is_sitting", B, ACTOR;
    IS_SLEEPING = "is_sleeping", B, ACTOR;
    IS_SNEAKING = "is_sneaking", B, ACTOR;
    IS_SNEEZING = "is_sneezing", B, ACTOR;
    IS_SNIFFING = "is_sniffing", B, ACTOR;
    IS_SONIC_BOOM = "is_sonic_boom", B, ACTOR;
    IS_SPECTATOR = "is_spectator", B, ACTOR => args(0, 0);
    IS_SPRINTING = "is_sprinting", B, ACTOR;
    IS_STACKABLE = "is_stackable", B, ACTOR;
    IS_STALKING = "is_stalking", B, ACTOR;
    IS_STANDING = "is_standing", B, ACTOR;
    IS_STUNNED = "is_stunned", B, ACTOR;
    IS_SWIMMING = "is_swimming", B, ACTOR;
    IS_TAMED = "is_tamed", B, ACTOR;
    IS_TRANSFORMING = "is_transforming", B, ACTOR;
    IS_USING_ITEM = "is_using_item", B, ACTOR;
    IS_WALL_CLIMBING = "is_wall_climbing", B, ACTOR;
    ITEM_IN_USE_DURATION = "item_in_use_duration", F, ACTOR;
    ITEM_IS_CHARGED = "item_is_charged", B, ACTOR => args(0, 1);
    ITEM_MAX_USE_DURATION = "item_max_use_duration", F, ACTOR;
    ITEM_REMAINING_USE_DURATION = "item_remaining_use_duration", F, ACTOR => split(1);
    ITEM_SLOT_TO_BONE_NAME = "item_slot_to_bone_name", HASH, ACTOR => args(1, 1);
    KEY_FRAME_LERP_TIME = "key_frame_lerp_time", F, RENDER => client();
    KINETIC_WEAPON_DAMAGE_DURATION = "kinetic_weapon_damage_duration", F, ITEM;
    KINETIC_WEAPON_DELAY = "kinetic_weapon_delay", F, ITEM;
    KINETIC_WEAPON_DISMOUNT_DURATION = "kinetic_weapon_dismount_duration", F, ITEM;
    KINETIC_WEAPON_KNOCKBACK_DURATION = "kinetic_weapon_knockback_duration", F, ITEM;
    LAST_FRAME_TIME = "last_frame_time", F, CLIENT_STATE => args(0, 1).client();
    LAST_HIT_BY_PLAYER = "last_hit_by_player", B, ACTOR;
    LAST_INPUT_MODE_IS_ANY = "last_input_mode_is_any", F, CLIENT_STATE => min_args(1).client();
    LEASHED_ENTITY_COUNT = "leashed_entity_count", F, ACTOR;
    LIE_AMOUNT = "lie_amount", F, ACTOR.union(RENDER);
    LIFE_SPAN = "life_span", F, ACTOR;
    LIFE_TIME = "life_time", F, RENDER => client();
    LOD_INDEX = "lod_index", F, CAMERA => client();
    LOG = "log", F, NONE => args(1, 1);
    MAIN_HAND_ITEM_MAX_DURATION = "main_hand_item_max_duration", F, ACTOR;
    MAIN_HAND_ITEM_USE_DURATION = "main_hand_item_use_duration", F, ACTOR;
    MARK_VARIANT = "mark_variant", F, ACTOR;
    MAX_DURABILITY = "max_durability", F, ITEM => args(0, 0);
    MAX_HEALTH = "max_health", F, ACTOR => args(0, 0);
    MAX_TRADE_TIER = "max_trade_tier", F, ACTOR;
    MAXIMUM_FRAME_TIME = "maximum_frame_time", F, CLIENT_STATE => args(0, 1).client();
    MINIMUM_FRAME_TIME = "minimum_frame_time", F, CLIENT_STATE => args(0, 1).client();
    MODEL_SCALE = "model_scale", F, RENDER => client();
    MODIFIED_DISTANCE_MOVED = "modified_distance_moved", F, ACTOR.union(RENDER);
    MODIFIED_MOVE_SPEED = "modified_move_speed", F, ACTOR;
    MODIFIED_SWING_DURATION = "modified_swing_duration", F, ACTOR;
    MOON_BRIGHTNESS = "moon_brightness", F, ACTOR => args(0, 0);
    MOON_PHASE = "moon_phase", F, ACTOR => args(0, 0);
    MOVEMENT_DIRECTION = "movement_direction", F, ACTOR => args(1, 1);
    NOISE = "noise", F, WORLD_GEN => world_gen();
    ON_FIRE_TIME = "on_fire_time", F, ACTOR;
    OUT_OF_CONTROL = "out_of_control", B, ACTOR;
    OVERLAY_ALPHA = "overlay_alpha", F, ACTOR;
    OWNER_IDENTIFIER = "owner_identifier", HASH, ACTOR => args(0, 0).default(EmptyString);
    PLAYER_LEVEL = "player_level", F, ACTOR => args(0, 0);
    POSITION = "position", F, ACTOR => args(1, 1);
    POSITION_DELTA = "position_delta", F, ACTOR => args(1, 1);
    PREVIOUS_SQUISH_VALUE = "previous_squish_value", F, ACTOR;
    PROPERTY = "property", F, ACTOR => args(1, 1);
    RELATIVE_BLOCK_HAS_ALL_TAGS = "relative_block_has_all_tags", B, ACTOR.union(BLOCK) => min_args(1);
    RELATIVE_BLOCK_HAS_ANY_TAG = "relative_block_has_any_tag", B, ACTOR.union(BLOCK) => min_args(1);
    REMAINING_DURABILITY = "remaining_durability", F, ITEM => args(0, 0);
    RIDE_BODY_X_ROTATION = "ride_body_x_rotation", F, ACTOR => args(0, 0);
    RIDE_BODY_Y_ROTATION = "ride_body_y_rotation", F, ACTOR => args(0, 0);
    RIDE_HEAD_X_ROTATION = "ride_head_x_rotation", F, ACTOR => args(0, 0);
    RIDE_HEAD_Y_ROTATION = "ride_head_y_rotation", F, ACTOR => args(0, 1);
    RIDER_BODY_X_ROTATION = "rider_body_x_rotation", F, ACTOR => args(1, 1);
    RIDER_BODY_Y_ROTATION = "rider_body_y_rotation", F, ACTOR => args(1, 1);
    RIDER_HEAD_X_ROTATION = "rider_head_x_rotation", F, ACTOR => args(1, 1);
    RIDER_HEAD_Y_ROTATION = "rider_head_y_rotation", F, ACTOR => args(1, 2);
    ROLL_COUNTER = "roll_counter", F, ACTOR;
    ROTATION_TO_CAMERA = "rotation_to_camera", F, CLIENT_STATE => args(1, 1).client();
    SCOREBOARD = "scoreboard", F, ACTOR => args(1, 1).server();
    SERVER_MEMORY_TIER = "server_memory_tier", F, ACTOR => args(0, 0).server();
    SHAKE_ANGLE = "shake_angle", F, ACTOR;
    SHAKE_TIME = "shake_time", F, ACTOR;
    SHIELD_BLOCKING_BOB = "shield_blocking_bob", F, ACTOR => args(0, 0);
    SHOW_BOTTOM = "show_bottom", B, ACTOR;
    SIT_AMOUNT = "sit_amount", F, ACTOR.union(RENDER);
    SKIN_ID = "skin_id", F, ACTOR;
    SLEEP_ROTATION = "sleep_rotation", F, ACTOR => args(0, 0);
    SNEEZE_COUNTER = "sneeze_counter", F, ACTOR;
    SPELLCOLOR = "spellcolor", STRUCT, ACTOR => default(StructRgba0);
    STANDING_SCALE = "standing_scale", F, ACTOR.union(RENDER);
    STATE_TIME = "state_time", F, RENDER => args(0, 0).client();
    STRUCTURAL_INTEGRITY = "structural_integrity", F, ACTOR => args(0, 0);
    SURFACE_PARTICLE_COLOR = "surface_particle_color", STRUCT, ACTOR.union(BLOCK) => args(0, 0).split(11).client();
    SURFACE_PARTICLE_TEXTURE_COORDINATE = "surface_particle_texture_coordinate", STRUCT, ACTOR.union(BLOCK) => args(0, 0).split(11);
    SURFACE_PARTICLE_TEXTURE_SIZE = "surface_particle_texture_size", STRUCT, ACTOR.union(BLOCK) => args(0, 0).split(11);
    SWELL_AMOUNT = "swell_amount", F, ACTOR;
    SWELLING_DIR = "swelling_dir", F, ACTOR;
    SWIM_AMOUNT = "swim_amount", F, ACTOR => args(0, 0);
    TAIL_ANGLE = "tail_angle", F, ACTOR;
    TARGET_X_ROTATION = "target_x_rotation", F, CLIENT_STATE => client();
    TARGET_Y_ROTATION = "target_y_rotation", F, CLIENT_STATE => client();
    TEXTURE_FRAME_INDEX = "texture_frame_index", F, ACTOR;
    TICKS_SINCE_LAST_KINETIC_WEAPON_HIT = "ticks_since_last_kinetic_weapon_hit", F, ACTOR => default(FloatNeg1);
    TIME_OF_DAY = "time_of_day", F, ACTOR => args(0, 0);
    TIME_SINCE_LAST_VIBRATION_DETECTION = "time_since_last_vibration_detection", F, ACTOR => args(0, 0).client().default(FloatNeg1);
    TIME_STAMP = "time_stamp", F, ACTOR;
    TIMER_FLAG_1 = "timer_flag_1", B, ACTOR => args(0, 0);
    TIMER_FLAG_2 = "timer_flag_2", B, ACTOR => args(0, 0);
    TIMER_FLAG_3 = "timer_flag_3", B, ACTOR => args(0, 0);
    TOTAL_EMITTER_COUNT = "total_emitter_count", F, CLIENT_STATE => client();
    TOTAL_PARTICLE_COUNT = "total_particle_count", F, CLIENT_STATE => client();
    TOUCH_ONLY_AFFECTS_HOTBAR = "touch_only_affects_hotbar", F, CLIENT_STATE => args(0, 0).client();
    TRADE_TIER = "trade_tier", F, ACTOR;
    UNHAPPY_COUNTER = "unhappy_counter", F, NONE;
    VARIANT = "variant", F, ACTOR;
    VERTICAL_SPEED = "vertical_speed", F, ACTOR;
    WALK_DISTANCE = "walk_distance", F, ACTOR;
    WING_FLAP_POSITION = "wing_flap_position", F, ACTOR;
    WING_FLAP_SPEED = "wing_flap_speed", F, ACTOR;
    YAW_SPEED = "yaw_speed", F, ACTOR;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::QueryAdmission;
    use crate::version::{ExperimentMask, RawVersion};
    use std::collections::{BTreeMap, BTreeSet};

    fn client() -> &'static QueryCatalog {
        crate::stdlib::queries(Side::Client)
    }

    fn get(name: &str) -> &'static QueryDecl {
        client()
            .get(name)
            .unwrap_or_else(|| panic!("{name} is not a standard query"))
    }

    fn windows(name: &str) -> Vec<(i16, i16)> {
        get(name)
            .shape()
            .ranges
            .as_slice()
            .iter()
            .map(|r| (r.first().as_i16(), r.last().as_i16()))
            .collect()
    }

    #[test]
    fn every_row_is_a_valid_declaration() {
        let decls = ROWS.iter().map(|(name, shape)| {
            QueryDecl::new(name, shape.clone()).unwrap_or_else(|error| panic!("{name}: {error}"))
        });
        assert_eq!(
            QueryCatalog::new(Side::Client, decls).map(|catalog| catalog.len()),
            Ok(ROWS.len())
        );
        for side in [Side::Client, Side::Server] {
            assert_eq!(catalog(side, None).len(), ROWS.len());
        }
    }

    /// 319 names: 315 in every supported release, four added by later releases.
    #[test]
    fn counts_and_names() {
        assert_eq!(ROWS.len(), 319);
        for side in [Side::Client, Side::Server] {
            assert_eq!(crate::stdlib::queries(side).len(), 319);
            assert_eq!(crate::stdlib::queries(side).side(), side);
        }
        assert_eq!(query::ABOVE_TOP_SOLID, "query.above_top_solid");
        assert_eq!(
            client().iter().next().map(QueryDecl::name),
            Some(query::ABOVE_TOP_SOLID)
        );
        assert_eq!(query::BLOCK_STATE, "query.block_state");
        assert_eq!(query::FUSE_TIME, "query.fuse_time");
        for (row, decl) in ROWS.iter().zip(client()) {
            assert_eq!(row.0, decl.name());
            assert_eq!(
                client().get_suffix(decl.suffix()).map(QueryDecl::name),
                Some(decl.name())
            );
        }
        let names: BTreeSet<&str> = client().iter().map(QueryDecl::name).collect();
        assert_eq!(names.len(), 319);
    }

    #[test]
    fn the_rows_are_sorted_by_name() {
        for pair in ROWS.windows(2) {
            assert!(
                pair[0].0 < pair[1].0,
                "{} is listed before {}",
                pair[0].0,
                pair[1].0
            );
        }
    }

    #[test]
    fn the_client_and_the_server_catalogues_declare_the_same_queries() {
        let server = crate::stdlib::queries(Side::Server);
        assert!(client().iter().zip(server).all(|(a, b)| a == b));
    }

    #[test]
    fn return_types() {
        let kind = |ty: ReturnType| {
            [
                (ReturnType::FLOAT, "float"),
                (ReturnType::BOOL, "bool"),
                (ReturnType::STRING, "string"),
                (ReturnType::ACTOR, "actor"),
                (ReturnType::ACTOR_ARRAY, "actor array"),
                (ReturnType::STRUCT, "struct"),
                (ReturnType::MATRIX, "matrix"),
            ]
            .into_iter()
            .find(|&(k, _)| k == ty)
            .map(|(_, name)| name)
            .expect("one kind")
        };
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for decl in client()
            .iter()
            .filter(|d| d.shape().first_release.is_none())
        {
            *counts.entry(kind(decl.shape().returns)).or_default() +=
                decl.shape().ranges.as_slice().len();
        }
        assert_eq!(
            counts,
            BTreeMap::from([
                ("actor", 1),
                ("actor array", 1),
                ("bool", 152),
                ("float", 151),
                ("matrix", 5),
                ("string", 4),
                ("struct", 7)
            ])
        );
        assert_eq!(
            get(query::COMBINE_ENTITIES).shape().returns,
            ReturnType::ACTOR_ARRAY
        );
        assert_eq!(
            get(query::EQUIPPED_ITEM_IS_ATTACHABLE).shape().returns,
            ReturnType::ACTOR
        );
    }

    #[test]
    fn later_releases_and_the_dedicated_server() {
        let fuse = get(query::FUSE_TIME);
        assert_eq!(fuse.shape().first_release, Some(Version::new(1, 26, 30)));
        assert_eq!(
            (
                fuse.sets(),
                fuse.shape().returns,
                fuse.shape().reads,
                fuse.shape().default_return
            ),
            (
                QuerySetMask::DEFAULT,
                ReturnType::FLOAT,
                Reads::ACTOR,
                DefaultReturn::Float0
            )
        );
        assert_eq!(fuse.args(), Arity::ANY);
        assert_eq!(windows(query::FUSE_TIME), [(0, 13)]);
        for (name, set) in [
            (query::HAS_ALL_BIOME_TAGS, QuerySetMask::WORLD_GEN),
            (query::HAS_ANY_BIOME_TAGS, QuerySetMask::WORLD_GEN),
            (query::HEAD_IS_IN_WATER, QuerySetMask::DEFAULT),
        ] {
            let decl = get(name);
            assert_eq!(
                decl.shape().first_release,
                Some(Version::new(1, 26, 50)),
                "{name}"
            );
            assert_eq!(
                (decl.sets(), decl.shape().returns),
                (set, ReturnType::BOOL),
                "{name}"
            );
        }
        let later: Vec<&str> = client()
            .iter()
            .filter(|d| d.shape().first_release.is_some())
            .map(QueryDecl::name)
            .collect();
        assert_eq!(
            later,
            [
                query::FUSE_TIME,
                query::HAS_ALL_BIOME_TAGS,
                query::HAS_ANY_BIOME_TAGS,
                query::HEAD_IS_IN_WATER
            ]
        );
        let absent: Vec<&str> = client()
            .iter()
            .filter(|d| !d.on_dedicated_server())
            .map(QueryDecl::name)
            .collect();
        assert_eq!(absent, [query::IS_ON_SCREEN]);
    }

    #[test]
    fn no_subject_defaults() {
        let exceptions = BTreeMap::from([
            (query::ARMOR_COLOR_SLOT, DefaultReturn::Float1),
            (
                query::TIME_SINCE_LAST_VIBRATION_DETECTION,
                DefaultReturn::FloatNeg1,
            ),
            (
                query::TICKS_SINCE_LAST_KINETIC_WEAPON_HIT,
                DefaultReturn::FloatNeg1,
            ),
            (query::OWNER_IDENTIFIER, DefaultReturn::EmptyString),
            (query::GET_EQUIPPED_ITEM_NAME, DefaultReturn::EmptyString),
            (query::COMBINE_ENTITIES, DefaultReturn::EmptyActorArray),
            (query::SPELLCOLOR, DefaultReturn::StructRgba0),
        ]);
        for decl in client() {
            assert_eq!(
                decl.shape().default_return,
                exceptions
                    .get(decl.name())
                    .copied()
                    .unwrap_or(DefaultReturn::Float0),
                "{}",
                decl.name()
            );
        }
    }

    #[test]
    fn sides_and_subjects() {
        let server: Vec<&str> = client()
            .iter()
            .filter(|d| d.shape().side == QuerySide::Server)
            .map(QueryDecl::name)
            .collect();
        assert_eq!(
            server,
            [
                query::ANGER_LEVEL,
                query::SCOREBOARD,
                query::SERVER_MEMORY_TIER
            ]
        );
        assert_ne!(get(query::IS_LOCAL_PLAYER).shape().side, QuerySide::Server);
        for name in [
            query::CLIENT_MAX_RENDER_DISTANCE,
            query::CLIENT_MEMORY_TIER,
            query::GET_PACK_SETTING,
            query::GRAPHICS_MODE_IS_ANY,
            query::HEARTBEAT_PHASE,
            query::IS_PACK_SETTING_ENABLED,
            query::IS_PACK_SETTING_SELECTED,
            query::LAST_INPUT_MODE_IS_ANY,
            query::SURFACE_PARTICLE_COLOR,
            query::TIME_SINCE_LAST_VIBRATION_DETECTION,
            query::TOUCH_ONLY_AFFECTS_HOTBAR,
            query::TARGET_X_ROTATION,
            query::BONE_AABB,
        ] {
            assert_eq!(get(name).shape().side, QuerySide::CLIENT, "{name}");
        }
        for name in [
            query::SURFACE_PARTICLE_TEXTURE_COORDINATE,
            query::SURFACE_PARTICLE_TEXTURE_SIZE,
        ] {
            assert_eq!(get(name).shape().side, QuerySide::BOTH, "{name}");
        }
        assert_eq!(
            get(query::TARGET_X_ROTATION).shape().reads,
            Reads::CLIENT_STATE
        );
        assert_eq!(get(query::IS_FIRST_PERSON).shape().reads, Reads::RENDER);
        assert_eq!(get(query::NOISE).shape().reads, Reads::WORLD_GEN);
        assert!(get(query::APPROX_EQ).shape().reads.is_empty());
        assert_eq!(
            get(query::SURFACE_PARTICLE_COLOR).shape().reads,
            Reads::ACTOR.union(Reads::BLOCK)
        );
        assert_eq!(
            get(query::DISTANCE_FROM_CAMERA).shape().reads,
            Reads::CAMERA
        );
        assert_eq!(get(query::BLOCK_FACE).shape().reads, Reads::VARIABLES);
        assert!(
            client()
                .iter()
                .filter(|d| d.shape().reads.contains(Reads::ACTOR))
                .count()
                > 200
        );
    }

    #[test]
    fn set_sizes() {
        let mut ranges_per_set: BTreeMap<&str, usize> = BTreeMap::new();
        let mut names_per_set: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for decl in client()
            .iter()
            .filter(|d| d.shape().first_release.is_none())
        {
            for range in decl.shape().ranges.as_slice() {
                let set = range.sets().name().expect("one built-in set per range");
                *ranges_per_set.entry(set).or_default() += 1;
                names_per_set.entry(set).or_default().insert(decl.name());
            }
        }
        assert_eq!(
            ranges_per_set,
            BTreeMap::from([("default", 315), ("tags", 2), ("world_gen", 4)])
        );
        let sizes: BTreeMap<&str, usize> =
            names_per_set.iter().map(|(k, v)| (*k, v.len())).collect();
        assert_eq!(
            sizes,
            BTreeMap::from([("default", 309), ("tags", 2), ("world_gen", 4)])
        );
        assert_eq!(
            names_per_set["tags"],
            BTreeSet::from([query::ALL_TAGS, query::ANY_TAG])
        );
        assert_eq!(
            names_per_set["world_gen"],
            BTreeSet::from([
                query::ABOVE_TOP_SOLID,
                query::HAS_BIOME_TAG,
                query::HEIGHTMAP,
                query::NOISE
            ])
        );
        // With the later releases: world_gen has six, default 311.
        assert_eq!(
            client()
                .iter()
                .filter(|d| d.sets() == QuerySetMask::WORLD_GEN)
                .count(),
            6
        );
        assert_eq!(
            client()
                .iter()
                .filter(|d| d.sets() == QuerySetMask::DEFAULT)
                .count(),
            311
        );
    }

    #[test]
    fn version_windows() {
        let two = [
            (query::ITEM_REMAINING_USE_DURATION, (0, 1), (2, 13)),
            (query::CAPE_FLAP_AMOUNT, (0, 7), (8, 13)),
            (query::SURFACE_PARTICLE_COLOR, (0, 11), (12, 13)),
            (
                query::SURFACE_PARTICLE_TEXTURE_COORDINATE,
                (0, 11),
                (12, 13),
            ),
            (query::SURFACE_PARTICLE_TEXTURE_SIZE, (0, 11), (12, 13)),
            (query::IS_CARRYING_BLOCK, (0, 12), (13, 13)),
        ];
        for (name, old, new) in two {
            assert_eq!(windows(name), [old, new], "{name}");
            for raw in -1..=13 {
                let expected = match raw {
                    raw if raw < 0 => None,
                    raw if raw <= old.1 => Some(0),
                    _ => Some(1),
                };
                assert_eq!(
                    get(name).resolve(
                        RawVersion(raw),
                        &QueryAdmission::Sets(QuerySetMask::BUILTIN),
                        ExperimentMask::empty()
                    ),
                    expected,
                    "{name} at {raw}"
                );
            }
        }
        assert_eq!(
            client()
                .iter()
                .filter(|d| d.shape().ranges.as_slice().len() == 2)
                .count(),
            6
        );
        let removed = [
            (query::BLOCK_PROPERTY, 9),
            (query::HAS_BLOCK_PROPERTY, 9),
            (query::DASH_COOLDOWN_PROGRESS, 10),
            (query::IS_SCENTING, 10),
            (query::IS_RISING, 10),
            (query::IS_FEELING_HAPPY, 10),
        ];
        for (name, last) in removed {
            assert_eq!(windows(name), [(0, last)], "{name}");
        }
        let special: BTreeSet<&str> = two
            .iter()
            .map(|(name, ..)| *name)
            .chain(removed.iter().map(|(name, _)| *name))
            .collect();
        for decl in client().iter().filter(|d| !special.contains(d.name())) {
            assert_eq!(windows(decl.name()), [(0, 13)], "{}", decl.name());
        }
    }

    #[test]
    fn argument_counts_and_experiments() {
        for decl in client() {
            let args = decl.args();
            assert!(
                args.max().is_none_or(|max| max >= args.min()),
                "{}",
                decl.name()
            );
            assert_eq!(
                decl.shape().experiments,
                ExperimentMask::empty(),
                "{}: no standard query needs an experiment",
                decl.name()
            );
        }
        assert_eq!(get(query::RIDE_BODY_X_ROTATION).args(), Arity::exactly(0));
        assert_eq!(get(query::IS_NAME_ANY).args(), Arity::at_least(1));
        assert_eq!(get(query::CAPE_FLAP_AMOUNT).args(), Arity::ANY);
    }
}
